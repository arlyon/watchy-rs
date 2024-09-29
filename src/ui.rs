use embedded_fonts::BdfTextStyle;
use embedded_graphics::{mono_font::MonoTextStyleBuilder, prelude::*, text::Text};
use epd_waveshare::{epd1in54::Display1in54, prelude::*};
use esp_hal::{gpio::GpioPin, peripherals::ADC1, prelude::*};
use futures::{pin_mut, StreamExt};

use core::cell::RefCell;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
use embassy_sync::blocking_mutex::{raw::NoopRawMutex, Mutex};
use epd_waveshare::epd1in54_v2::Epd1in54;
use esp_hal::{
    delay::Delay,
    gpio::{Input, Level, Output, Pull},
    peripherals::SPI2,
    spi::master::Spi,
};

use crate::{BatteryStatusDriver, GlobalTime};

const TIMEZONE: time::UtcOffset = match time::UtcOffset::from_hms(1, 0, 0) {
    Ok(v) => v,
    Err(_) => panic!("Bad value"),
};

#[embassy_executor::task]
pub async fn drive_display(
    spi: SPI2,
    sck: GpioPin<47>,
    miso: GpioPin<46>,
    mosi: GpioPin<48>,
    cs: GpioPin<33>,
    dc: GpioPin<34>,
    reset: GpioPin<35>,
    busy: GpioPin<36>,
    global_time: GlobalTime,
    mut delay: Delay,
    battery_adc: GpioPin<9>,
    charge_pin: GpioPin<10>,
    adc: ADC1,
) {
    let pin_spi_edp_cs = Output::new(cs, Level::Low);
    let pin_edp_dc = Output::new(dc, Level::Low);
    let pin_edp_reset = Output::new(reset, Level::Low);
    let pin_edp_busy = Input::new(busy, Pull::Up);

    let spi = Spi::new(spi, 2.MHz(), esp_hal::spi::SpiMode::Mode0)
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
        Some(1_000),
    )
    .unwrap();

    // every 5 renders we should use the full LUT
    let lut_loop = [
        Some(RefreshLut::Full),
        Some(RefreshLut::Quick),
        None,
        None,
        None,
    ];

    let mut battery = BatteryStatusDriver::new(battery_adc, charge_pin, adc);

    loop {
        defmt::info!("starting draw loop");

        // render now, and every 60 seconds
        let updates =
            futures::stream::once(async { global_time.get_time() }).chain(global_time.minutes());

        let lut_loop = futures::stream::iter(lut_loop).cycle();

        let draw_patterns = updates.zip(lut_loop);
        pin_mut!(draw_patterns);

        while let Some((update, lut)) = draw_patterns.next().await {
            defmt::info!("drawing");
            let update = i64::try_from(update / 1_000_000).unwrap();
            let date = time::OffsetDateTime::from_unix_timestamp(update)
                .unwrap()
                .to_offset(TIMEZONE);

            defmt::info!(
                "{} -> date is {}/{}/{} {} {}",
                update,
                date.year(),
                u8::from(date.month()),
                date.day(),
                date.hour(),
                date.minute()
            );

            epd.wake_up(&mut spi, &mut delay).unwrap();

            if let Some(lut) = lut {
                epd.set_lut(&mut spi, &mut delay, Some(lut)).unwrap();
            };

            let style = BdfTextStyle::new(
                &crate::fonts::space_mono::FONT_SPACEM_ITALICN_ITALIC_REGULAR,
                Color::Black,
            );

            let battery_style = MonoTextStyleBuilder::new()
                .font(&embedded_graphics::mono_font::ascii::FONT_7X14_BOLD)
                .text_color(Color::Black)
                .build();

            // Use display graphics from embedded-graphics
            let display = {
                let mut display = Display1in54::default();
                display.clear(Color::White).unwrap();

                {
                    let mut string = heapless::String::<8>::new();
                    if date.hour() < 10 {
                        ufmt::uwrite!(string, "0{}", date.hour()).unwrap();
                    } else {
                        ufmt::uwrite!(string, "{}", date.hour()).unwrap();
                    };
                    let _ = Text::new(&string, Point::new(20, 50), style).draw(&mut display);
                }
                {
                    let _ = Text::new(":", Point::new(85, 45), style).draw(&mut display);
                }
                {
                    let mut string = heapless::String::<8>::new();
                    if date.minute() < 10 {
                        ufmt::uwrite!(string, "0{}", date.minute()).unwrap();
                    } else {
                        ufmt::uwrite!(string, "{}", date.minute()).unwrap();
                    };
                    let _ = Text::new(&string, Point::new(115, 50), style).draw(&mut display);
                }

                {
                    let bat = battery.status().await.unwrap();
                    let mut string = heapless::String::<20>::new();

                    ufmt::uwrite!(
                        string,
                        "{}mV ({}%) {}",
                        bat.voltage(),
                        bat.percentage(),
                        match battery.charging().await {
                            true => "+",
                            false => "",
                        }
                    )
                    .unwrap();
                    let _ =
                        Text::new(&string, Point::new(60, 195), battery_style).draw(&mut display);
                }

                display
            };

            epd.update_frame(&mut spi, display.buffer(), &mut delay)
                .unwrap();

            // Display updated frame
            // epd.update_frame(&mut spi, display.buffer(), &mut delay)
            //     .unwrap();
            epd.display_frame(&mut spi, &mut delay).unwrap();

            defmt::info!("sleeping display");

            // Set the EPD to sleep
            epd.sleep(&mut spi, &mut delay).unwrap();
        }
    }
}
