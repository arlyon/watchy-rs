# watchy-rs

A firmware for the watchy smartwatch written in rust.
This firmware targets watchy v3, which is based on the
ESP32S3 chip and the xtensa rust toolchain. It uses
[`embassy-rs`](https://github.com/embassy-rs/embassy),
an async executor for embedded rust, to power its tasks
and attempts to use async drivers as much as possible
for better performance and power consumption.

The firmware is `#[no_std]` and requires allocations
only for the wifi stack. It uses the [`esp-hal`](https://github.com/esp-rs/esp-hal)
crate to provide implementations of the [`embedded-hal`](https://github.com/rust-embedded/embedded-hal)
and [`embedded-hal-async`](https://github.com/rust-embedded/embedded-hal-async)
traits for the ESP32S3 chip.

Embassy will automatically put the chip into modem
sleep when it is idle. As of now, battery life is
in the 4 hour range, but that is only because we
do not use the deep or light sleep modes.

Energy consumption is roughtly 50Ma, but is expected
to be closer to 1Ma average once full sleep works.

## Roadmap

- [x] Async wifi connection
- [x] Async ntp time sync
- [x] Async display driver
- [x] Battery status reading
- [x] Accelerometer reading
- [x] Automatic modem sleep
- [x] RTC time syncing
- [x] Buttons and vibration
- [ ] Light sleep between updates
- [ ] Deep sleep between updates

# tests

See https://github.com/esp-rs/esp-hal/tree/main/hil-test

# face inspo

- https://github.com/Prokuon/watchy-starfield
- https://github.com/sqfmi/watchy-pipboy
- https://github.com/vtu-dog/qlock

https://gitlab.com/claudiomattera/esp32c3-embassy
