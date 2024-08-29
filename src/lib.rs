#![no_std]

use defmt::write;
use esp_hal::{peripherals::LPWR, reset::SleepSource};

mod battery;

pub use battery::{BatteryStatus, BatteryStatusDriver};

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Button {
    BottomLeft,
    TopLeft,
    TopRight,
    BottomRight,
}

// TODO set these channels
const RTCIO_GPIO4_CHANNEL: u32 = 1 << 10;
const RTCIO_GPIO25_CHANNEL: u32 = 1 << 6;
const RTCIO_GPIO26_CHANNEL: u32 = 1 << 7;
const RTCIO_GPIO35_CHANNEL: u32 = 1 << 5;

fn get_ext1_wakeup_button(rtc_cntl: &LPWR) -> Result<Button, u32> {
    // TODO when esp32_hal lets you read the wakeup status, it'd be nice to use that
    // instead of using unsafe.
    let wakeup_bits = rtc_cntl.ext_wakeup1_status().read().bits();

    match wakeup_bits {
        RTCIO_GPIO26_CHANNEL => Ok(Button::BottomLeft),
        RTCIO_GPIO25_CHANNEL => Ok(Button::TopLeft),
        RTCIO_GPIO35_CHANNEL => Ok(Button::TopRight),
        RTCIO_GPIO4_CHANNEL => Ok(Button::BottomRight),
        _ => Err(wakeup_bits),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum WakeupCause {
    /// First boot or manual reset from serial monitor
    Reset,
    /// The PCF8563 RTC told us to wake up
    ExternalRtcAlarm,
    /// One of the buttons was pressed
    ButtonPress(Button),
    // Probably shouldn't happen since we only set those pins for waking up
    // TODO turn into Error
    UnknownExt1(u32),
    // Probably shouldn't happen
    // TODO turn into Error
    Unknown(SleepSource),
}

impl defmt::Format for WakeupCause {
    fn format(&self, fmt: defmt::Formatter) {
        match self {
            WakeupCause::Reset => write!(fmt, "reset"),
            WakeupCause::ExternalRtcAlarm => write!(fmt, "external rtc"),
            WakeupCause::ButtonPress(_) => write!(fmt, "button press"),
            WakeupCause::UnknownExt1(_) => write!(fmt, "unknown ext"),
            WakeupCause::Unknown(_) => write!(fmt, "unknown"),
        }
    }
}

pub fn get_wakeup_cause(rtc_cntl: &LPWR) -> WakeupCause {
    let cause = esp_hal::reset::get_wakeup_cause();

    match cause {
        SleepSource::Ext0 => WakeupCause::ExternalRtcAlarm,
        SleepSource::Ext1 => match get_ext1_wakeup_button(rtc_cntl) {
            Ok(button) => WakeupCause::ButtonPress(button),
            Err(mask) => WakeupCause::UnknownExt1(mask),
        },
        SleepSource::Undefined => WakeupCause::Reset,
        _ => WakeupCause::Unknown(cause),
    }
}
