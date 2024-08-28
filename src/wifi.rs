//! wifi
//!
//! This module adds wifi support. To use it, start the wifi task and the net_task.
//! The net_task drives the wifi stack while wifi connects to an IP and does stuff.

use core::str::FromStr;
use embassy_executor::Spawner;
use embassy_net::udp::PacketMetadata;
use embassy_net::{Config, Stack, StackResources};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::peripherals::RADIO_CLK;
use esp_hal::rng::Rng;
use esp_hal::timer::{ErasedTimer, PeriodicTimer};
use esp_hal::{
    clock::Clocks,
    peripherals::{RNG, WIFI},
};
use esp_wifi::{
    initialize,
    wifi::{
        ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
        WifiState,
    },
    EspWifiInitFor,
};
use sntpc::NtpResult;
use static_cell::make_static;

pub enum MessageType {
    TimeUpdate(&'static Signal<CriticalSectionRawMutex, TimeResponse>),
    WeatherUpdate(&'static Signal<CriticalSectionRawMutex, WeatherResponse>),
}

pub type TimeResponse = Option<NtpResult>;
pub struct WeatherResponse {}

/// A bus for coordinating commands that can be actioned by the network task
static NETWORK_BUS: Channel<CriticalSectionRawMutex, MessageType, 10> = Channel::new();

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

#[embassy_executor::task]
pub async fn wifi(
    timer: PeriodicTimer<ErasedTimer>,
    rng: RNG,
    radio_clock_control: RADIO_CLK,
    clocks: &'static Clocks<'_>,
    wifi: WIFI,
    spawner: Spawner,
) {
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(rng),
        radio_clock_control,
        clocks,
    )
    .unwrap();

    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

    let config = Config::dhcpv4(Default::default());

    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = &*make_static!(Stack::new(
        wifi_interface,
        config,
        make_static!(StackResources::<3>::new()),
        seed
    ));

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    defmt::info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            defmt::info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    loop {
        let msg = NETWORK_BUS.receive().await;
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
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    defmt::info!("start connection task");
    // defmt::info!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        if esp_wifi::wifi::get_wifi_state() == WifiState::StaConnected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
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
            Ok(_) => defmt::info!("Wifi connected!"),
            Err(e) => {
                defmt::info!("Failed to connect to wifi");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}
