use core::cell::RefCell;

use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embedded_graphics::geometry::Point;
use embedded_graphics::prelude::Primitive;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, PrimitiveStyleBuilder};
use embedded_graphics::Drawable;
use epd_waveshare::color::Color;
use epd_waveshare::epd1in54::{Display1in54, Epd1in54};
use epd_waveshare::prelude::WaveshareDisplay;

use esp_hal::clock::Clocks;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputPin, Output, OutputPin};
use esp_hal::peripheral::Peripheral;
use esp_hal::peripherals::SPI2;
use esp_hal::prelude::*;
use esp_hal::spi::master::Spi;
use esp_hal::spi::FullDuplexMode;

pub struct WatchyDisplay<'a, SPI, P1, P2, P3>
where
    P1: OutputPin,
    P2: OutputPin,
    P3: InputPin,
{
    epd: Epd1in54<SPI, Input<'a, P3>, Output<'a, P1>, Output<'a, P2>, Delay>,
    spi: SPI,
    delay: &'a mut Delay,
}

type DefaultSpi<'a, P> = SpiDevice<'a, NoopRawMutex, Spi<'a, SPI2, FullDuplexMode>, P>;

impl<'a, P1, P2, P3, P> WatchyDisplay<'a, DefaultSpi<'a, P>, P1, P2, P3>
where
    P1: OutputPin,
    P2: OutputPin,
    P3: InputPin,
    P: embedded_hal::digital::OutputPin,
{
    pub fn new(
        spi2: SPI2,
        clocks: &Clocks,
        delay: &'a mut Delay,
        pin_spi_sck: impl Peripheral<P = impl OutputPin> + 'a,
        pin_spi_miso: impl Peripheral<P = impl InputPin> + 'a,
        _pin_spi_mosi: impl Peripheral<P = impl OutputPin>,
        pin_spi_edp_cs: P,
        pin_edp_dc: Output<'a, P1>,
        pin_edp_reset: Output<'a, P2>,
        pin_edp_busy: Input<'a, P3>,
    ) -> Result<WatchyDisplay<'a, DefaultSpi<'a, P>, P1, P2, P3>, ()> {
        let spi = Spi::new(spi2, 20.MHz(), esp_hal::spi::SpiMode::Mode0, clocks)
            .with_sck(pin_spi_sck)
            .with_miso(pin_spi_miso);

        let spi = Mutex::new(RefCell::new(spi));

        let mut spi = SpiDevice::new(&spi, pin_spi_edp_cs);
        let epd = Epd1in54::new(
            &mut spi_dev,
            pin_edp_busy.into(),
            pin_edp_dc.into(),
            pin_edp_reset.into(),
            delay,
            None,
        )
        .unwrap();

        Ok(WatchyDisplay { epd, spi, delay })
    }
}

impl<'a, SPI, P1, P2, P3> WatchyDisplay<'a, SPI, P1, P2, P3>
where
    P1: OutputPin,
    P2: OutputPin,
    P3: InputPin,
    SPI: embedded_hal::spi::SpiDevice,
{
    pub fn draw_test(&mut self) -> Result<(), SPI::Error> {
        // Use display graphics from embedded-graphics
        let mut display = Display1in54::default();

        // Use embedded graphics for drawing a line
        let style = PrimitiveStyleBuilder::new()
            .stroke_color(Color::Black)
            .stroke_width(1)
            .build();
        let _ = Line::new(Point::new(0, 120), Point::new(0, 295))
            .into_styled(style)
            .draw(&mut display);
        let _ = Circle::with_center(Point::new(50, 50), 50)
            .into_styled(PrimitiveStyle::with_fill(Color::White))
            .draw(&mut display);

        // Display updated frame
        self.epd
            .update_frame(&mut self.spi, &display.buffer(), &mut self.delay)?;
        self.epd.display_frame(&mut self.spi, &mut self.delay)?;

        // Set the EPD to sleep
        self.epd.sleep(&mut self.spi, &mut self.delay)?;
        Ok(())
    }
}
