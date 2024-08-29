//! Battery status using the ADC.

use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    gpio::Gpio10,
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
    adc1_pin: esp_hal::analog::adc::AdcPin<esp_hal::gpio::GpioPin<10>, ADC1, ()>,
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
        battery_pins: Gpio10,
        adc: P,
    ) -> Self {
        // Create ADC instances
        let mut adc1_config = AdcConfig::new();
        let adc1_pin = adc1_config.enable_pin_with_cal(battery_pins, Attenuation::Attenuation11dB);
        let adc1 = Adc::new(adc, adc1_config);

        Self { adc1_pin, adc1 }
    }

    /// Retrieve the battery status by sampling the ADC.
    pub fn status(&mut self) -> Result<BatteryStatus, ()> {
        let Ok(val) = nb::block!(self.adc1.read_oneshot(&mut self.adc1_pin)) else {
            return Err(());
        };
        Ok(BatteryStatus(u32::from(val)))
    }
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
