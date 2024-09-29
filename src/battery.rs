//! Battery status using the ADC.

use esp_hal::{
    analog::adc::{Adc, AdcCalLine, AdcConfig, Attenuation},
    gpio::{ErasedPin, GpioPin, Input, Level, Pull},
    peripherals::ADC1,
    prelude::nb,
};

/// Represents a battery status.
pub struct BatteryStatus(u32);
impl BatteryStatus {
    /// Returns the battery voltage in mV.
    pub fn voltage(&self) -> u32 {
        self.0
    }

    /// Returns the charge percentage of the battery.
    pub fn percentage(&self) -> u8 {
        // NOTE: The percentage calculation is linear from 3400 mV to 4200 mV
        self.0
            .saturating_sub(3400)
            .saturating_mul(100)
            .div_euclid(4200 - 3400)
            .min(100)
            .try_into()
            .unwrap()
    }
}

/// Driver to retrieve the battery status.
///
/// The battery voltage sampled using an
/// [ADC](https://en.wikipedia.org/wiki/Analog-to-digital_converter)
/// peripheral on the ESP32.
pub struct BatteryStatusDriver<'d> {
    adc1_pin: esp_hal::analog::adc::AdcPin<esp_hal::gpio::GpioPin<9>, ADC1, AdcCalLine<ADC1>>,
    chrg_pin: esp_hal::analog::adc::AdcPin<esp_hal::gpio::GpioPin<10>, ADC1, AdcCalLine<ADC1>>,
    // chrg_pin: Input<'d, ErasedPin>,
    adc1: Adc<'d, ADC1>,
}
impl<'d> BatteryStatusDriver<'d> {
    /// Setup a new battery status driver.
    ///
    /// # Example
    /// ```no_run
    /// let peripherals = watchy::hal::peripherals::Peripherals::take().unwrap();
    /// let pin_sets = watchy::pins::Sets::new(peripherals.pins);
    /// let mut battery_staus_driver =
    ///     watchy::battery::BatteryStatusDriver::new(pin_sets.battery, peripherals.adc1).unwrap();
    /// ```
    pub fn new<P: esp_hal::peripheral::Peripheral<P = ADC1> + 'd>(
        battery_pin: GpioPin<9>,
        chrg_pin: GpioPin<10>,
        adc: P,
    ) -> Self {
        // Create ADC instances
        let mut adc1_config = AdcConfig::new();
        let adc1_pin = adc1_config.enable_pin_with_cal::<GpioPin<9>, AdcCalLine<ADC1>>(
            battery_pin,
            Attenuation::Attenuation11dB,
        );
        let chrg_pin = adc1_config.enable_pin_with_cal::<GpioPin<10>, AdcCalLine<ADC1>>(
            chrg_pin,
            Attenuation::Attenuation11dB,
        );
        let adc1 = Adc::new(adc, adc1_config);

        // let chrg_pin = Input::new(chrg_pin, Pull::Up);

        Self {
            adc1_pin,
            adc1,
            chrg_pin,
        }
    }

    /// Retrieve the battery status by sampling the ADC.
    pub async fn status(&mut self) -> Result<BatteryStatus, ()> {
        let Ok(voltage) = crate::block_embassy!(self.adc1.read_oneshot(&mut self.adc1_pin)) else {
            return Err(());
        };

        // adjust voltage based on the algo in the watchy firmware
        let voltage = voltage as f32 * ((360.0 + 100.0) / 360.0);
        let voltage = voltage as u32;

        Ok(BatteryStatus(voltage))
    }

    /// The battery is charging if the charge pin is low.
    pub async fn charging(&mut self) -> bool {
        // let level = self.chrg_pin.get_level();
        // defmt::info!("reading charge pin {:?}", level);
        // level

        let Ok(voltage) = crate::block_embassy!(self.adc1.read_oneshot(&mut self.chrg_pin)) else {
            return false;
        };

        // over 3000 is charging
        voltage > 3000
    }
}

/// Turns the non-blocking expression `$e` into a blocking operation.
///
/// This is accomplished by continuously calling the expression `$e` until it no
/// longer returns `Error::WouldBlock`
///
/// # Input
///
/// An expression `$e` that evaluates to `nb::Result<T, E>`
///
/// # Output
///
/// - `Ok(t)` if `$e` evaluates to `Ok(t)`
/// - `Err(e)` if `$e` evaluates to `Err(nb::Error::Other(e))`
#[macro_export]
macro_rules! block_embassy {
    ($e:expr) => {
        loop {
            #[allow(unreachable_patterns)]
            match $e {
                Err(nb::Error::Other(e)) =>
                {
                    #[allow(unreachable_code)]
                    break Err(e)
                }
                Err(nb::Error::WouldBlock) => {
                    embassy_futures::yield_now().await;
                }
                Ok(x) => break Ok(x),
            }
        }
    };
}

// TODO figure this out

// pin_project_lite::pin_project! {
// struct AdcReadFuture<'a, ADCI, PIN> {
//     #[pin]
//     adc: &'a mut Adc<'a, ADCI>,
//     #[pin]
//     pin: &'a mut AdcPin<PIN, ADCI, ()>,
// }
// }

// impl<'a, ADCI, PIN> core::future::Future for AdcReadFuture<'a, ADCI, PIN>
// where
//     ADCI: esp_hal::analog::adc::RegisterAccess,
//     PIN: AdcChannel,
// {
//     type Output = Result<u16, ()>;

//     fn poll(
//         self: core::pin::Pin<&mut Self>,
//         cx: &mut core::task::Context<'_>,
//     ) -> core::task::Poll<Self::Output> {
//         let mut self2 = self.project();
//         match self2.adc.read_oneshot(&mut self2.pin) {
//             Ok(res) => core::task::Poll::Ready(Ok(res)),
//             Err(nb::Error::WouldBlock) => core::task::Poll::Pending,
//             Err(nb::Error::Other(e)) => core::task::Poll::Ready(Err(e)),
//         }
//     }
// }
