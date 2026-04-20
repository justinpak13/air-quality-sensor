use embassy_rp::spi::{self, Blocking, ClkPin, Instance, MosiPin, Spi};
use embassy_rp::{Peri, gpio::Output};
use embassy_time::{Delay, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};

use crate::{INIT_PROGRESS, SensorValue};
use sh1106::{Builder, prelude::*};

// starting point for display
const X: i32 = 10;
const Y: i32 = 10;

// buffer for display
const BASE_TEXT: &str = "eCO2:       ppm\n               \n\nTVOC:       ppb\n               ";

pub struct OLEDDisplay<'a, I: Instance> {
    display: GraphicsMode<SpiInterface<Spi<'a, I, Blocking>, Output<'a>, Output<'a>>>,
    text_char_buffer: [u8; 128],
    style: MonoTextStyle<'a, BinaryColor>,
    res: Output<'a>,
}

impl<'a, I: Instance> OLEDDisplay<'a, I> {
    pub fn new(
        mosi: Peri<'a, impl MosiPin<I>>,
        clk: Peri<'a, impl ClkPin<I>>,
        dc: Output<'a>,
        cs: Output<'a>,
        res: Output<'a>,
        inner_spi: Peri<'a, I>,
    ) -> Self {
        // set up and fill character buffer
        let mut text_char_buffer = [b' '; 128];
        for (i, c) in BASE_TEXT.chars().enumerate() {
            text_char_buffer[i] = c as u8;
        }

        // text style for display
        let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

        // communication setup for display
        let spi = Spi::new_blocking_txonly(inner_spi, clk, mosi, spi::Config::default());

        // struct for working with oled
        let display: GraphicsMode<_> = Builder::new()
            .with_rotation(DisplayRotation::Rotate180)
            .connect_spi(spi, dc, cs) // pass cs here instead of NoOutputPin
            .into();

        OLEDDisplay {
            display,
            text_char_buffer,
            style,
            res,
        }
    }

    pub async fn initialize(&mut self) {
        self.display.reset(&mut self.res, &mut Delay).unwrap();
        self.display.init().unwrap();

        Rectangle::new(Point::zero(), Size::new(128, 64))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut self.display)
            .unwrap();

        self.display.flush().unwrap();

        self.display_text_from_str("hello!\nbooting up...", X, Y);
        Timer::after_secs(3).await;
    }

    pub async fn loading(&mut self) {
        let mut buf = [b' '; 50];
        let text = "Loading:   %\n[          ]";
        for (i, c) in text.chars().enumerate() {
            buf[i] = c as u8;
        }
        let bar_start = 14;
        let number_start = 10;
        loop {
            let pct = INIT_PROGRESS.wait().await;
            let mut temp = pct;

            for i in 0..(pct / 10) as usize {
                buf[i + bar_start] = b'*';
            }

            let mut number_index = number_start;

            while temp > 0 {
                let digit = temp % 10;
                temp /= 10;
                buf[number_index] = b'0' + digit as u8;
                number_index -= 1;
            }

            self.display_text_from_str(
                core::str::from_utf8(&buf).unwrap_or("something went wrong"),
                X,
                Y,
            );

            if pct >= 100 {
                break;
            }
        }
    }

    pub async fn startup_text(&mut self) {
        // once all initialized - startup text
        self.display_text_from_str("hardware checks\ncompleted\n\nsensor warming\nup", X, Y);
        Timer::after_secs(3).await;
    }

    fn display_text_from_str(&mut self, text: &str, x: i32, y: i32) {
        self.display.clear();
        self.display
            .flush()
            .unwrap_or_else(|e| log::info!("Error with OLED display: {e:?}"));

        match Text::new(text, Point::new(x, y), self.style).draw(&mut self.display) {
            Ok(_) => {}
            Err(e) => log::info!("Error with OLED display: {e:?}"),
        }

        self.display
            .flush()
            .unwrap_or_else(|e| log::info!("Error with OLED display: {e:?}"));
    }

    fn display_info(&mut self) {
        self.display.clear();

        Text::new(
            core::str::from_utf8(&self.text_char_buffer).unwrap_or("issue with\ntext buffer"),
            Point::new(X, Y),
            self.style,
        )
        .draw(&mut self.display)
        .unwrap();

        self.display
            .flush()
            .unwrap_or_else(|e| log::info!("Error with OLED display: {e:?}"));
    }

    pub fn update_buffer(&mut self, eco2: &SensorValue, tvoc: &SensorValue) {
        // will have to change if base text gets updated
        const ECO2_VALUE_START: usize = 6;
        const TVOC_VALUE_START: usize = 39;

        const ECO2_TEXT_START: usize = 16;
        const TVOC_TEXT_START: usize = 49;

        self.update_values_in_buffer(eco2.value, ECO2_VALUE_START);
        self.update_values_in_buffer(tvoc.value, TVOC_VALUE_START);

        self.update_text_in_buffer(&eco2.message, ECO2_TEXT_START);
        self.update_text_in_buffer(&tvoc.message, TVOC_TEXT_START);
    }
    pub fn update_display(&mut self) {
        self.display_info();
    }

    fn update_values_in_buffer(&mut self, value: u16, start: usize) {
        let mut temp_value = value;

        let mut buffer: [u8; 5] = [b'0', b' ', b' ', b' ', b' '];
        let mut index = 0;

        while temp_value > 0 {
            let digit = temp_value % 10;
            temp_value /= 10;

            buffer[index] = b'0' + digit as u8;

            index += 1;
        }

        buffer.swap(0, 4);
        buffer.swap(1, 3);

        for (index, c) in buffer.into_iter().enumerate() {
            self.text_char_buffer[start + index] = c;
        }
    }

    fn update_text_in_buffer(&mut self, text: &[u8; 15], start: usize) {
        for (index, c) in text.iter().enumerate() {
            self.text_char_buffer[start + index] = *c;
        }
    }
}
