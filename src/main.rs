#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![deny(clippy::unwrap_used)]
#![feature(impl_trait_in_assoc_type)]

use esp_backtrace as _;
use esp_println as _;

use esp_hal::prelude::*;

use async_debounce::Debouncer;
use bma423::{Bma423, FeatureInterruptStatus, InterruptDirection, PowerControlFlag, Uninitialized};
use core::future;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, Either4};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::{Duration, Timer};
use embedded_hal_async::digital::Wait;
use esp_hal::clock::{ClockControl, Clocks};
use esp_hal::delay::Delay;
use esp_hal::gpio::{
    Gpio0, Gpio10, Gpio13, Gpio14, Gpio17, Gpio6, Gpio7, Gpio8, Input, Io, Level, Output, Pull,
};
use esp_hal::i2c::I2C;
use esp_hal::interrupt::Priority;
use esp_hal::peripherals::{Peripherals, ADC1, I2C0};
use esp_hal::system::SystemControl;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::{ErasedTimer, OneShotTimer, PeriodicTimer};
use esp_hal::Blocking;
use esp_hal_embassy::InterruptExecutor;
use static_cell::StaticCell;
use watchy_rs::GlobalTime;

static CLOCK: StaticCell<Clocks> = StaticCell::new();
static I2C_G: StaticCell<I2C0> = StaticCell::new();
static TIMERS: StaticCell<[OneShotTimer<ErasedTimer>; 1]> = StaticCell::new();

static VIBRATION: StaticCell<Output<Gpio17>> = StaticCell::new();

/// Run the OS
///
/// We have two task spawners, a low priority one and a high prio one which responds to
/// things like buttons.
#[main]
async fn main(low_prio_spawner: Spawner) {
    let peripherals = Peripherals::take();
    let cause = watchy_rs::get_wakeup_cause(&peripherals.LPWR);
    defmt::info!("starting due to {:?}", cause);

    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = CLOCK.init(ClockControl::max(system.clock_control).freeze());
    let delay = Delay::new(clocks);
    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);

    let embassy_timers = {
        let timg0 = TimerGroup::new(peripherals.TIMG0, clocks, None);
        let timer0: ErasedTimer = timg0.timer0.into();
        let timers = [OneShotTimer::new(timer0)];
        TIMERS.init(timers)
    };

    esp_hal_embassy::init(clocks, embassy_timers);

    {
        defmt::info!("starting button / vibro handler");
        static EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();
        let executor =
            InterruptExecutor::new(system.software_interrupt_control.software_interrupt2);
        let executor = EXECUTOR.init(executor);
        let spawner = executor.start(Priority::Priority3);
        let vibration_motor = Output::new(io.pins.gpio17, Level::Low);
        let vibration_motor = VIBRATION.init(vibration_motor);
        spawner.must_spawn(handle_buttons(
            io.pins.gpio7,
            io.pins.gpio6,
            io.pins.gpio0,
            io.pins.gpio8,
            io.pins.gpio14,
            io.pins.gpio13,
            vibration_motor,
        ));
    }

    {
        let wifi_timer = {
            let timg1 = TimerGroup::new(peripherals.TIMG1, clocks, None);
            let timer0: ErasedTimer = timg1.timer0.into();
            PeriodicTimer::new(timer0)
        };

        low_prio_spawner.must_spawn(watchy_rs::wifi(
            wifi_timer,
            peripherals.RNG,
            peripherals.RADIO_CLK,
            clocks,
            peripherals.WIFI,
            low_prio_spawner,
        ));
    }

    let global_time = GlobalTime::new();

    low_prio_spawner.must_spawn(watchy_rs::drive_display(
        peripherals.SPI2,
        io.pins.gpio47,
        io.pins.gpio46,
        io.pins.gpio48,
        io.pins.gpio33,
        io.pins.gpio34,
        io.pins.gpio35,
        io.pins.gpio36,
        clocks,
        global_time,
        delay,
        io.pins.gpio9,
        peripherals.ADC1,
    ));

    {
        let i2c = I2C_G.init(peripherals.I2C0);
        let i2c0 = I2C::new(i2c, io.pins.gpio12, io.pins.gpio11, 400.kHz(), clocks, None);
        let accel = Bma423::new(
            i2c0,
            bma423::Config {
                bandwidth: bma423::AccelConfigBandwidth::CicAvg8,
                range: bma423::AccelRange::Range2g,
                performance_mode: bma423::AccelConfigPerfMode::CicAvg,
                sample_rate: bma423::AccelConfigOdr::Odr100,
            },
        );
        low_prio_spawner.must_spawn(handle_accel(accel, delay));
    }

    let time = watchy_rs::get_time().await;
    if let Some(time) = time {
        if time.offset < 0 {
            panic!("invalid response");
        }
        global_time.init_offset(time.offset as u64);
        defmt::info!("seconds: {}", time.offset);
    } else {
        defmt::info!("couldn't get time");
    }
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
    _acc_int_2: Gpio13,
    vibration: &'static mut Output<'static, Gpio17>,
) {
    let vibration_signal = embassy_sync::signal::Signal::<NoopRawMutex, _>::new();

    let debounce_time = embassy_time::Duration::from_millis(5);
    let mut button_1 = Debouncer::new(Input::new(p1, Pull::None), debounce_time);
    let mut button_2 = Debouncer::new(Input::new(p2, Pull::None), debounce_time);
    let mut button_3 = Debouncer::new(Input::new(p3, Pull::None), debounce_time);
    let mut button_4 = Debouncer::new(Input::new(p4, Pull::None), debounce_time);
    let mut interrupt = Debouncer::new(Input::new(acc_int_1, Pull::Up), debounce_time);

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

            let res = embassy_futures::select::select(buttons, future::pending::<()>()).await;

            match res {
                Either::First(a) => {
                    vibration_signal.signal(60);
                    match a {
                        Either4::First(_) => {
                            defmt::info!("button 1 pressed");
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
                Either::Second(_) => {
                    vibration_signal.signal(60);
                }
            }
        }
    };

    embassy_futures::join::join3(drive_vibro, drive_buttons, drive_accel).await;
}
