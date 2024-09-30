//! wifi
//!
//! This module adds wifi support. To use it, start the wifi task and the net_task.
//! The net_task drives the wifi stack while wifi connects to an IP and does stuff.

use core::str::FromStr;
use embassy_executor::Spawner;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::{Config, Stack, StackResources};
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
use reqwless::client::HttpClient;
use reqwless::headers::ContentType;
use reqwless::request::{Method, RequestBuilder};
use static_cell::make_static;

static SSID: &str = "Lavenderhaugen";
const PASSWORD: &str = include_str!("../wifi-password.txt");

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

    let mut rx_buffer = [0; 4096];

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

    let state: TcpClientState<1, 1024, 1024> = TcpClientState::new();
    let client = TcpClient::new(stack, &state);
    let mut client = HttpClient::new(&client, &crate::dns::StaticDns);

    client
        .request(Method::POST, "http://10.13.1.179:8080")
        .await
        .unwrap()
        .body(())
        .content_type(ContentType::ApplicationOctetStream)
        .send(&mut rx_buffer)
        .await
        .unwrap();
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
