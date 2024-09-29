//! wifi
//!
//! This module adds wifi support. To use it, start the wifi task and the net_task.
//! The net_task drives the wifi stack while wifi connects to an IP and does stuff.

use core::str::FromStr;
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_net::udp::PacketMetadata;
use embassy_net::{Config, Stack, StackResources};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::peripherals::RADIO_CLK;
use esp_hal::peripherals::{RNG, WIFI};
use esp_hal::rng::Rng;
use esp_hal::timer::{ErasedTimer, PeriodicTimer};
use esp_wifi::{
    initialize,
    wifi::{
        ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
        WifiState,
    },
    EspWifiInitFor,
};
use sntpc::NtpResult;
use static_cell::StaticCell;

use crate::sticky_signal::StickySignal;

pub enum MessageType {
    TimeUpdate(&'static Signal<CriticalSectionRawMutex, TimeResponse>),
    WeatherUpdate(&'static Signal<CriticalSectionRawMutex, WeatherResponse>),
}

pub type TimeResponse = Option<NtpResult>;
pub struct WeatherResponse {}

/// A bus for coordinating commands that can be actioned by the network task
static NETWORK_BUS: Channel<CriticalSectionRawMutex, MessageType, 10> = Channel::new();
static ENABLE_NETWORK: StickySignal<CriticalSectionRawMutex, bool, 4> =
    StickySignal::new_with_name("enable_network");

static SSID: &str = "NOW1QQ9L";
const PASSWORD: &str = include_str!("../wifi-password.txt");

// new requests should just reuse existing values
static TIME_SIGNAL: Signal<CriticalSectionRawMutex, TimeResponse> = Signal::new();
static WEATHER_SIGNAL: Signal<CriticalSectionRawMutex, WeatherResponse> = Signal::new();

pub async fn get_time() -> TimeResponse {
    // todo: avoid making already fulfilled requests
    let (time, _) = embassy_futures::join::join(
        TIME_SIGNAL.wait(),
        NETWORK_BUS.send(MessageType::TimeUpdate(&TIME_SIGNAL)),
    )
    .await;

    time
}

pub async fn get_weather() -> WeatherResponse {
    // todo: avoid making already fulfilled requests
    let (weather, _) = embassy_futures::join::join(
        WEATHER_SIGNAL.wait(),
        NETWORK_BUS.send(MessageType::WeatherUpdate(&WEATHER_SIGNAL)),
    )
    .await;

    weather
}

static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
static WIFI_STACK: StaticCell<Stack<WifiDevice<'static, WifiStaDevice>>> = StaticCell::new();

#[embassy_executor::task]
pub async fn wifi(
    timer: PeriodicTimer<'static, ErasedTimer>,
    rng: RNG,
    radio_clock_control: RADIO_CLK,
    wifi: WIFI,
    spawner: Spawner,
) {
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(rng),
        radio_clock_control,
    )
    .unwrap();

    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

    let config = Config::dhcpv4(Default::default());

    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = Stack::new(
        wifi_interface,
        config,
        STACK_RESOURCES.init(StackResources::<3>::new()),
        seed,
    );
    let stack = WIFI_STACK.init(stack);

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(stack)).ok();

    loop {
        let msg = NETWORK_BUS.receive().await;
        ENABLE_NETWORK.signal(true);

        loop {
            if stack.is_link_up() {
                break;
            }
            Timer::after(Duration::from_millis(100)).await;
        }

        defmt::info!("Waiting to get IP address...");
        loop {
            if let Some(config) = stack.config_v4() {
                defmt::info!("Got IP: {}", config.address);
                break;
            }
            Timer::after(Duration::from_millis(100)).await;
        }

        match msg {
            MessageType::TimeUpdate(sig) => {
                let mut rx_meta = [PacketMetadata::EMPTY; 16];
                let mut rx_buffer = [0; 4096];
                let mut tx_meta = [PacketMetadata::EMPTY; 16];
                let mut tx_buffer = [0; 4096];
                let mut socket = embassy_net::udp::UdpSocket::new(
                    stack,
                    &mut rx_meta,
                    &mut rx_buffer,
                    &mut tx_meta,
                    &mut tx_buffer,
                );
                socket.bind(9400).unwrap();
                defmt::info!("getting time");
                let res = crate::time::get_time(socket).await;
                defmt::info!("sending result {}", res.is_some());
                sig.signal(res);
            }
            _ => unimplemented!(),
        }

        if NETWORK_BUS.is_empty() {
            ENABLE_NETWORK.signal(false);
        }
    }
}

/// Connect and disconnect to wifi depending on if there are requests queued.
#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    defmt::info!("start connection task");
    let mut connect_failures = 0;
    const MAX_CONNECT_FAILURES: usize = 3;
    loop {
        defmt::trace!("wifi loop");
        if esp_wifi::wifi::get_wifi_state() == WifiState::StaConnected {
            match select(
                ENABLE_NETWORK.wait_for("wifi loop disabled", |val| (!val).then_some(false)),
                controller.wait_for_event(WifiEvent::StaDisconnected),
            )
            .await
            {
                // disconnect
                Either::First(_) => {
                    defmt::info!("stopping wifi");
                    controller.stop().await.unwrap();
                }
                // we disconnected involuntarily, attempt to reconnect
                Either::Second(_) => {
                    Timer::after(Duration::from_millis(5000)).await;
                }
            };
        }
        if !matches!(controller.is_started(), Ok(true)) {
            // if we haven't started, wait until we should start
            ENABLE_NETWORK
                .wait_for("wait to start wifi", |val| val.then_some(true))
                .await;

            let client_config = Configuration::Client(ClientConfiguration {
                ssid: heapless::String::from_str(SSID).unwrap(),
                password: heapless::String::from_str(PASSWORD).unwrap(),

                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            defmt::info!("Starting wifi");
            let data = controller.start().await;
            defmt::info!("Wifi started! {:?}", data);
        }
        defmt::info!("About to connect...");

        match controller.connect().await {
            Ok(()) => defmt::info!("Wifi connected!"),
            Err(e) => {
                defmt::info!("Failed to connect to wifi {:?}", e);
                connect_failures += 1;
                if connect_failures > MAX_CONNECT_FAILURES {
                    defmt::info!("Shutting down wifi");
                    ENABLE_NETWORK.signal(false);
                    controller.stop().await.unwrap();
                    connect_failures = 0;
                }
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    // wait for network to be enabled, then select on it being disabled

    loop {
        defmt::trace!("net loop");
        ENABLE_NETWORK
            .wait_for("net loop enabled", |val| val.then_some(true))
            .await;
        defmt::trace!("network enabled");
        match select(
            ENABLE_NETWORK.wait_for("net loop disabled", |val| (!val).then_some(false)),
            stack.run(),
        )
        .await
        {
            Either::First(_) => {
                defmt::info!("stopping net task");
            }
            Either::Second(_) => {}
        }
    }
}
