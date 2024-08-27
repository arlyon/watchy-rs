use embedded_graphics::{
    mono_font::MonoTextStyleBuilder,
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
    text::Text,
};
use epd_waveshare::{epd1in54::Display1in54, prelude::*};
use esp_hal::prelude::*;

use core::cell::RefCell;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
use embassy_sync::blocking_mutex::{raw::NoopRawMutex, Mutex};
use epd_waveshare::epd1in54_v2::Epd1in54;
use esp_hal::{
    clock::Clocks,
    delay::Delay,
    gpio::{Gpio33, Gpio34, Gpio35, Gpio36, Gpio46, Gpio47, Gpio48, Input, Level, Output, Pull},
    peripherals::SPI2,
    spi::master::Spi,
};

#[embassy_executor::task]
pub async fn drive_display(
    spi: SPI2,
    sck: Gpio47,
    miso: Gpio46,
    mosi: Gpio48,
    cs: Gpio33,
    dc: Gpio34,
    reset: Gpio35,
    busy: Gpio36,
    clocks: &'static Clocks<'static>,
    mut delay: Delay,
) {
    let pin_spi_edp_cs = Output::new(cs, Level::Low);
    let pin_edp_dc = Output::new(dc, Level::Low);
    let pin_edp_reset = Output::new(reset, Level::Low);
    let pin_edp_busy = Input::new(busy, Pull::Up);

    let spi = Spi::new(spi, 2.MHz(), esp_hal::spi::SpiMode::Mode0, clocks)
        .with_sck(sck)
        .with_miso(miso)
        .with_mosi(mosi);

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
    epd.update_frame(&mut spi, display.buffer(), &mut delay)
        .unwrap();
    epd.display_frame(&mut spi, &mut delay).unwrap();

    defmt::info!("sleeping display");

    // Set the EPD to sleep
    epd.sleep(&mut spi, &mut delay).unwrap();

    defmt::info!("done");
}
