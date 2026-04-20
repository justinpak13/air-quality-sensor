#![no_std]
#![no_main]
#![warn(clippy::pedantic)]
use core::cmp::Ordering;

mod air_sensor;
mod display;

use embassy_executor::Spawner;
use embassy_rp::block::ImageDef;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::pwm::{Pwm, SetDutyCycle};
use embassy_rp::{self as hal, i2c};
use embassy_rp::{peripherals::USB, usb};
use embassy_time::Timer;

use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

//Panic Handler
use panic_probe as _;
// Defmt Logging
use defmt_rtt as _;

use crate::air_sensor::AirQualitySensor;
use crate::display::OLEDDisplay;

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: ImageDef = hal::block::ImageDef::secure_exe();

// usb interrupt binding
embassy_rp::bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

#[embassy_executor::task]
async fn logger_task(usb: embassy_rp::Peri<'static, embassy_rp::peripherals::USB>) {
    let driver = embassy_rp::usb::Driver::new(usb, Irqs);

    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

static INIT_PROGRESS: Signal<CriticalSectionRawMutex, u8> = Signal::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // set up hardware connections
    let p = embassy_rp::init(Default::default());

    // pins for air quality sensor
    let sda = p.PIN_0;
    let scl = p.PIN_1;

    // pins for display
    let mosi = p.PIN_11;
    let clk = p.PIN_10;
    let dc = Output::new(p.PIN_8, Level::Low);
    let cs = Output::new(p.PIN_9, Level::High);
    let res = Output::new(p.PIN_15, Level::High);
    let inner_spi = p.SPI1;

    // set up pins for leds
    let (yellow, red) =
        Pwm::new_output_ab(p.PWM_SLICE2, p.PIN_20, p.PIN_21, Default::default()).split();
    let mut red = red.unwrap();
    let mut yellow = yellow.unwrap();
    let mut green = Pwm::new_output_b(p.PWM_SLICE1, p.PIN_19, Default::default());

    // communication setup
    // for air quality sensor
    let i2c = i2c::I2c::new_blocking(p.I2C0, scl, sda, i2c::Config::default());

    let mut oled_display = OLEDDisplay::new(mosi, clk, dc, cs, res, inner_spi);
    let mut air_sensor = AirQualitySensor::new(i2c);

    // logging for usb connection
    spawner.must_spawn(logger_task(p.USB));

    // initialization
    oled_display.initialize().await;

    let mut eco2 = SensorValue::new(MeasurementType::Eco2);
    let mut tvoc = SensorValue::new(MeasurementType::Tvoc);

    select(air_sensor.initialize(), oled_display.loading()).await;
    oled_display.startup_text().await;

    // variable to track whether redraw is needed
    let mut display_dirty = false;

    loop {
        Timer::after_millis(100).await;

        if air_sensor.is_data_ready()
            && let Some((eco2_reading, tvoc_reading)) = air_sensor.get_data()
        {
            if eco2_reading != eco2.value || tvoc_reading != tvoc.value {
                eco2.update(eco2_reading);
                tvoc.update(tvoc_reading);
                display_dirty = true;
            }

            log::info!("eCO2: {eco2_reading} ppm | TVOC: {tvoc_reading} ppb");
            log::info!(
                "eCO2 -> category: {:?}, percentage: {}",
                eco2.category,
                eco2.led_percentage
            );

            log::info!(
                "tvoc -> category: {:?}, percentage: {}",
                tvoc.category,
                tvoc.led_percentage
            );

            if display_dirty {
                oled_display.update_buffer(&eco2, &tvoc);
                oled_display.update_display();

                set_led(&eco2, &tvoc, &mut red, &mut yellow, &mut green);
                display_dirty = false;
            }
        }
    }
}

enum MeasurementType {
    Eco2,
    Tvoc,
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq)]
enum MeasurementCategory {
    Green,
    Yellow,
    Red,
}

pub struct SensorValue {
    value: u16,
    measurement_type: MeasurementType,
    category: MeasurementCategory,
    message: [u8; 15],
    led_percentage: u8,
}

impl SensorValue {
    fn new(measurement_type: MeasurementType) -> Self {
        match measurement_type {
            MeasurementType::Eco2 => SensorValue {
                value: 400,
                measurement_type,
                category: MeasurementCategory::Green,
                message: [b' '; 15],
                led_percentage: 100,
            },
            MeasurementType::Tvoc => SensorValue {
                value: 0,
                measurement_type,
                category: MeasurementCategory::Green,
                message: [b' '; 15],
                led_percentage: 100,
            },
        }
    }

    fn update(&mut self, new_value: u16) {
        self.value = new_value;

        let (new_message, new_category, new_percentage) = match self.measurement_type {
            MeasurementType::Eco2 => match self.value {
                v if v <= 600 => (
                    "Excellent",
                    MeasurementCategory::Green,
                    100 - ((v - 400) / 400) * 100,
                ),
                v if v <= 800 => (
                    "Good",
                    MeasurementCategory::Green,
                    100 - ((v - 400) / 400) * 100,
                ),
                v if v <= 1000 => ("Moderate", MeasurementCategory::Yellow, (v - 800) / (2)),
                v if v <= 1500 => ("Poor", MeasurementCategory::Red, (v - 1000) / 10),
                v if v <= 2000 => ("Very Poor", MeasurementCategory::Red, (v - 1000) / 10),
                v if v > 2000 => ("Hazardous", MeasurementCategory::Red, 100),
                _ => unreachable!(),
            },
            MeasurementType::Tvoc => match self.value {
                v if v <= 50 => ("Excellent", MeasurementCategory::Green, 100 - v),
                v if v <= 100 => ("Good", MeasurementCategory::Green, 100 - v),
                v if v <= 200 => ("Moderate", MeasurementCategory::Yellow, (v - 100)),
                v if v <= 300 => ("Poor", MeasurementCategory::Red, (v - 200) / 3),
                v if v <= 500 => ("Very Poor", MeasurementCategory::Red, (v - 200) / 3),
                v if v > 500 => ("Hazardous", MeasurementCategory::Red, 100),
                _ => unreachable!(),
            },
        };

        self.category = new_category;
        self.led_percentage = new_percentage.clamp(1, 100) as u8;

        let blank_spots = self.message.len() - new_message.len();

        for i in 0..blank_spots {
            self.message[i] = b' ';
        }

        for (i, c) in new_message.chars().enumerate() {
            self.message[i + blank_spots] = c as u8;
        }
    }
}

trait DynSetDutyCycle {
    fn set_duty_cycle_percent(&mut self, percent: u8);
}

impl<T: SetDutyCycle> DynSetDutyCycle for T {
    fn set_duty_cycle_percent(&mut self, percent: u8) {
        let _ = SetDutyCycle::set_duty_cycle_percent(self, percent);
    }
}

fn set_led(
    eco2_reading: &SensorValue,
    tvoc_reading: &SensorValue,
    red: &mut dyn DynSetDutyCycle,
    yellow: &mut dyn DynSetDutyCycle,
    green: &mut dyn DynSetDutyCycle,
) {
    let significant_reading = match eco2_reading.category.cmp(&tvoc_reading.category) {
        Ordering::Less => tvoc_reading,
        Ordering::Greater => eco2_reading,
        Ordering::Equal => {
            if eco2_reading.category == MeasurementCategory::Green {
                if eco2_reading.led_percentage < tvoc_reading.led_percentage {
                    eco2_reading
                } else {
                    tvoc_reading
                }
            } else {
                if eco2_reading.led_percentage < tvoc_reading.led_percentage {
                    tvoc_reading
                } else {
                    eco2_reading
                }
            }
        }
    };
    green.set_duty_cycle_percent(0);
    yellow.set_duty_cycle_percent(0);
    red.set_duty_cycle_percent(0);

    match significant_reading.category {
        MeasurementCategory::Green => {
            green.set_duty_cycle_percent(significant_reading.led_percentage);
        }
        MeasurementCategory::Yellow => {
            yellow.set_duty_cycle_percent(significant_reading.led_percentage);
        }
        MeasurementCategory::Red => {
            red.set_duty_cycle_percent(significant_reading.led_percentage);
        }
    }
}

// Program metadata for `picotool info`.
// This isn't needed, but it's recommended to have these minimal entries.
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"air_quality_sensor"),
    embassy_rp::binary_info::rp_program_description!(c"your program description"),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

// End of file
