#![no_std]
#![no_main]
#![feature(error_in_core)]
#![feature(type_alias_impl_trait)]
#![deny(clippy::unwrap_used)]

use esp_backtrace as _;
use esp_println as _;

use async_debounce::Debouncer;
use bma423::{Bma423, FeatureInterruptStatus, InterruptDirection, PowerControlFlag, Uninitialized};
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;

use embassy_futures::select::{Either, Either4};
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::text::Text;
use embedded_hal_async::digital::Wait;

use esp_hal::i2c::I2C;
use esp_hal::interrupt::Priority;
use esp_hal_embassy::InterruptExecutor;

use embedded_graphics::prelude::*;
use epd_waveshare::prelude::*;
use esp_hal::{prelude::*, Blocking};

use core::cell::RefCell;
use core::future;
use embassy_sync::blocking_mutex::Mutex;
use embedded_graphics::primitives::{Circle, PrimitiveStyle};
use epd_waveshare::epd1in54_v2::{Display1in54, Epd1in54};

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::clock::{ClockControl, Clocks};
use esp_hal::delay::Delay;
use esp_hal::gpio::{
    Gpio0, Gpio10, Gpio11, Gpio12, Gpio13, Gpio14, Gpio17, Gpio5, Gpio6, Gpio7, Gpio8, Input, Io,
    Level, Output, Pull,
};
use esp_hal::peripherals::{Peripherals, ADC1, I2C0};
use esp_hal::spi::master::Spi;
use esp_hal::system::SystemControl;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::{ErasedTimer, OneShotTimer, PeriodicTimer};
use static_cell::StaticCell;

// mod display;
//

pub const MSGS: usize = 50;
pub const SUBS: usize = 2;
pub const PUBS: usize = 1;

static CLOCK: StaticCell<Clocks> = StaticCell::new();
static BUS: StaticCell<PubSubChannel<NoopRawMutex, (Instant, ()), MSGS, SUBS, PUBS>> =
    StaticCell::new();
static I2C_G: StaticCell<I2C0> = StaticCell::new();
static TIMERS: StaticCell<[OneShotTimer<ErasedTimer>; 1]> = StaticCell::new();

static BUTTON_1: Mutex<CriticalSectionRawMutex, RefCell<Option<Input<'static, Gpio12>>>> =
    Mutex::new(RefCell::new(None));
static BUTTON_2: Mutex<CriticalSectionRawMutex, RefCell<Option<Input<'static, Gpio11>>>> =
    Mutex::new(RefCell::new(None));
static BUTTON_3: Mutex<CriticalSectionRawMutex, RefCell<Option<Input<'static, Gpio5>>>> =
    Mutex::new(RefCell::new(None));
static BUTTON_4: Mutex<CriticalSectionRawMutex, RefCell<Option<Input<'static, Gpio13>>>> =
    Mutex::new(RefCell::new(None));

static VIBRATION: StaticCell<Output<Gpio17>> = StaticCell::new();

/// Run the OS
///
/// We have two task spawners, a low priority one and a high prio one which responds to
/// things like buttons.
#[main]
async fn main(low_prio_spawner: Spawner) {
    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();
    let clocks = CLOCK.init(clocks);
    let mut delay = Delay::new(&clocks);
    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);

    let cause = watchy_rs::get_wakeup_cause(&peripherals.LPWR);
    defmt::info!("starting due to {:?}", cause);

    let bus = BUS.init(PubSubChannel::new());
    let timg0 = TimerGroup::new(peripherals.TIMG0, clocks, None);
    let timer0: ErasedTimer = timg0.timer0.into();

    let timer1 = {
        let timg1 = TimerGroup::new(peripherals.TIMG1, &clocks, None);
        let timer0: ErasedTimer = timg1.timer0.into();
        PeriodicTimer::new(timer0)
    };

    // let timer = PeriodicTimer::new(timer0);

    let timers = [OneShotTimer::new(timer0)];
    let timers = TIMERS.init(timers);
    esp_hal_embassy::init(clocks, timers);

    let i2c = I2C_G.init(peripherals.I2C0);

    let i2c0 = I2C::new(
        i2c,
        io.pins.gpio12,
        io.pins.gpio11,
        400.kHz(),
        &clocks,
        None,
    );

    let vibration_motor = Output::new(io.pins.gpio17, Level::Low);
    let vibration_motor = VIBRATION.init(vibration_motor);

    defmt::info!("CREATE BMA");

    let accel = Bma423::new(
        i2c0,
        bma423::Config {
            bandwidth: bma423::AccelConfigBandwidth::CicAvg8,
            range: bma423::AccelRange::Range2g,
            performance_mode: bma423::AccelConfigPerfMode::CicAvg,
            sample_rate: bma423::AccelConfigOdr::Odr100,
        },
    );

    // accel.

    defmt::info!("SPAWN TASKS");

    low_prio_spawner.must_spawn(handle_accel(accel, delay));
    // low_prio_spawner.must_spawn(watchy_rs::wifi(
    //     timer1,
    //     peripherals.RNG,
    //     peripherals.RADIO_CLK,
    //     clocks,
    //     peripherals.WIFI,
    //     low_prio_spawner,
    // ));

    static EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();
    let executor = InterruptExecutor::new(system.software_interrupt_control.software_interrupt2);
    let executor = EXECUTOR.init(executor);

    let spawner = executor.start(Priority::Priority3);
    spawner.must_spawn(handle_buttons(
        io.pins.gpio7,
        io.pins.gpio6,
        io.pins.gpio0,
        io.pins.gpio8,
        io.pins.gpio14,
        io.pins.gpio13,
        io.pins.gpio10,
        peripherals.ADC1,
        vibration_motor,
    ));

    defmt::info!("Spawning low-priority tasks");

    let spi2 = peripherals.SPI2;
    let pin_spi_sck = io.pins.gpio47;
    let pin_spi_miso = io.pins.gpio46;
    let pin_spi_mosi = io.pins.gpio48;
    let pin_spi_edp_cs = Output::new(io.pins.gpio33, Level::Low);
    let pin_edp_dc = Output::new(io.pins.gpio34, Level::Low);
    let pin_edp_reset = Output::new(io.pins.gpio35, Level::Low);
    let pin_edp_busy = Input::new(io.pins.gpio36, Pull::Up);

    let spi = Spi::new(spi2, 2.MHz(), esp_hal::spi::SpiMode::Mode0, clocks)
        .with_sck(pin_spi_sck)
        .with_miso(pin_spi_miso)
        .with_mosi(pin_spi_mosi);

    let spi = Mutex::<NoopRawMutex, _>::new(RefCell::new(spi));

    let mut spi = SpiDevice::new(&spi, pin_spi_edp_cs);
    let mut epd = Epd1in54::new(
        &mut spi,
        pin_edp_busy,
        pin_edp_dc,
        pin_edp_reset,
        &mut delay,
        None,
    )
    .unwrap();

    epd.wake_up(&mut spi, &mut delay).unwrap();

    defmt::info!("drawing");

    // clear the display
    epd.clear_frame(&mut spi, &mut delay).unwrap();
    epd.display_frame(&mut spi, &mut delay).unwrap();

    let style = MonoTextStyleBuilder::new()
        .font(&embedded_graphics::mono_font::ascii::FONT_7X14_BOLD)
        .text_color(Color::White)
        .background_color(Color::Black)
        .build();

    // Use display graphics from embedded-graphics
    let display = {
        let mut display = Display1in54::default();
        display.clear(Color::White).unwrap();

        let _ = Circle::with_center(Point::new(100, 100), 50)
            .into_styled(PrimitiveStyle::with_fill(Color::Black))
            .draw(&mut display);

        let _ = Text::new("FUCK", Point::new(87, 105), style).draw(&mut display);

        display
    };

    // Display updated frame
    epd.update_frame(&mut spi, &display.buffer(), &mut delay)
        .unwrap();
    epd.display_frame(&mut spi, &mut delay).unwrap();

    defmt::info!("sleeping display");

    // Set the EPD to sleep
    epd.sleep(&mut spi, &mut delay).unwrap();

    defmt::info!("done");
}

#[embassy_executor::task]
async fn handle_accel(
    accel: Bma423<I2C<'static, I2C0, Blocking>, Uninitialized>,
    mut delay: Delay,
) {
    let mut accel = accel.init(&mut delay).unwrap();
    accel
        .set_power_control(PowerControlFlag::Auxiliary)
        .unwrap();

    accel
        .set_interrupt_config(
            bma423::InterruptLine::Line1,
            InterruptDirection::Input(bma423::InterruptTriggerCondition::Edge),
        )
        .unwrap();

    let mut features = accel.edit_features().unwrap();
    features
        .set_tap_config(bma423::features::TapFeature::SingleTap, 3, true)
        .unwrap();
    features.write().unwrap();

    accel
        .map_feature_interrupt(
            bma423::InterruptLine::Line1,
            FeatureInterruptStatus::SingleTap,
            true,
        )
        .unwrap();

    loop {
        // -z is face up
        // +x is vertical
        // +y is rotated left
        let (x, y, z) = accel.accel_norm_int().unwrap();
        defmt::info!("ACCEL: x: {} y: {} z: {}", x, y, z);
        Timer::after(Duration::from_millis(1000 * 60 * 60)).await;
    }
}

/// Periodically print something.
#[embassy_executor::task]
async fn handle_buttons(
    p1: Gpio7,
    p2: Gpio6,
    p3: Gpio0,
    p4: Gpio8,
    acc_int_1: Gpio14,
    acc_int_2: Gpio13,
    stat: Gpio10,
    adc: ADC1,
    vibration: &'static mut Output<'static, Gpio17>,
) {
    let vibration_signal = embassy_sync::signal::Signal::<NoopRawMutex, _>::new();

    let debounce_time = embassy_time::Duration::from_millis(5);
    let mut button_1 = Debouncer::new(Input::new(p1, Pull::None), debounce_time);
    let mut button_2 = Debouncer::new(Input::new(p2, Pull::None), debounce_time);
    let mut button_3 = Debouncer::new(Input::new(p3, Pull::None), debounce_time);
    let mut button_4 = Debouncer::new(Input::new(p4, Pull::None), debounce_time);
    let mut interrupt = Debouncer::new(Input::new(acc_int_1, Pull::Up), debounce_time);

    let mut battery = watchy_rs::BatteryStatusDriver::new(stat, adc);

    defmt::info!("getting battery status");
    let status = battery.status().await.unwrap();
    defmt::info!("status: {:?}", status.voltage());

    let mut is_charging = false;

    let drive_accel = async {
        loop {
            interrupt.wait_for_any_edge().await.unwrap();
            defmt::info!("TAP")
        }
    };

    let drive_vibro = async {
        let mut vib_timeout = futures::future::Either::Left(future::pending());
        loop {
            match embassy_futures::select::select(vibration_signal.wait(), vib_timeout).await {
                Either::First(new_wait) => {
                    vibration.set_high();
                    vib_timeout =
                        futures::future::Either::Right(embassy_time::Timer::after_millis(new_wait))
                }
                Either::Second(_) => {
                    vibration.set_low();
                    vib_timeout = futures::future::Either::Left(future::pending())
                }
            }
        }
    };

    let drive_buttons = async {
        loop {
            let buttons = embassy_futures::select::select4(
                button_1.wait_for_falling_edge(),
                button_2.wait_for_falling_edge(),
                button_3.wait_for_falling_edge(),
                button_4.wait_for_falling_edge(),
            );

            let res = embassy_futures::select::select(
                buttons,
                // todo
                // charging.wait_for_falling_edge()
                future::pending::<()>(),
            )
            .await;

            match res {
                Either::First(a) => {
                    vibration_signal.signal(60);
                    match a {
                        Either4::First(_) => {
                            defmt::info!("charging: {}", is_charging);
                        }
                        Either4::Second(_) => {
                            defmt::info!("button 2 pressed");
                        }
                        Either4::Third(_) => {
                            defmt::info!("button 3 pressed");
                        }
                        Either4::Fourth(_) => {
                            defmt::info!("button 4 pressed");
                        }
                    }
                }
                Either::Second(b) => {
                    vibration_signal.signal(60);
                    is_charging = true
                }
            }
        }
    };

    embassy_futures::join::join3(drive_vibro, drive_buttons, drive_accel).await;
}
