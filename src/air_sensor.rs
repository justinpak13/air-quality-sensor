use embassy_rp::i2c::{I2c, Instance, Mode};
use embassy_time::Timer;

use crate::INIT_PROGRESS;

// CCS811 constants
const CCS811_ADDR: u8 = 0x5A;
const REG_HW_ID: u8 = 0x20;
const REG_STATUS: u8 = 0x00;
const REG_APP_START: u8 = 0xF4;
const REG_ALG_RESULT_DATA: u8 = 0x02;
const REG_MEAS_MODE: u8 = 0x01;

pub struct AirQualitySensor<'a, I: Instance, M: Mode> {
    buffer: [u8; 8],
    i2c: I2c<'a, I, M>,
}

impl<'a, I: Instance, M: Mode> AirQualitySensor<'a, I, M> {
    pub fn new(i2c: I2c<'a, I, M>) -> Self {
        AirQualitySensor {
            buffer: [0u8; 8],
            i2c,
        }
    }

    pub async fn initialize(&mut self) {
        Timer::after_millis(10).await;
        INIT_PROGRESS.signal(10);
        // initialization of air quality sensor
        match self
            .i2c
            .blocking_write_read(CCS811_ADDR, &[REG_HW_ID], &mut self.buffer)
        {
            Ok(()) => log::info!("hardware id should be 0x81: {}", self.buffer[0]),
            Err(e) => log::info!("I2C ERROR: {e:?}"),
        }
        Timer::after_millis(10).await;
        INIT_PROGRESS.signal(20);

        match self
            .i2c
            .blocking_write_read(CCS811_ADDR, &[REG_STATUS], &mut self.buffer)
        {
            Ok(()) => log::info!("STATUS: {:#x}", self.buffer[0]),
            Err(e) => log::info!("I2C ERROR: {e:?}"),
        }
        Timer::after_millis(10).await;
        INIT_PROGRESS.signal(30);

        match self.i2c.blocking_write(CCS811_ADDR, &[REG_APP_START]) {
            Ok(()) => log::info!("app started"),
            Err(e) => log::info!("error with starting app {e:?}"),
        }
        Timer::after_millis(10).await;
        INIT_PROGRESS.signal(40);

        match self
            .i2c
            .blocking_write_read(CCS811_ADDR, &[REG_STATUS], &mut self.buffer)
        {
            Ok(()) => log::info!("STATUS: {:#x}", self.buffer[0]),
            Err(e) => log::info!("I2C ERROR: {e:?}"),
        }
        Timer::after_millis(10).await;
        INIT_PROGRESS.signal(50);

        match self.i2c.blocking_write(CCS811_ADDR, &[REG_MEAS_MODE, 0x10]) {
            Ok(()) => log::info!("Measurement mode set"),
            Err(e) => log::info!("MEAS_MODE error: {e:?}"),
        }
        Timer::after_millis(10).await;
        INIT_PROGRESS.signal(80);

        // Confirm MEAS_MODE was written
        let mut mode = [0u8; 1];
        self.i2c
            .blocking_write_read(CCS811_ADDR, &[REG_MEAS_MODE], &mut mode)
            .unwrap();
        log::info!("MEAS_MODE readback: {:#04x}", mode[0]); // expect 0x10
        Timer::after_millis(10).await;
        INIT_PROGRESS.signal(100);
    }

    pub fn is_data_ready(&mut self) -> bool {
        match self
            .i2c
            .blocking_write_read(CCS811_ADDR, &[REG_STATUS], &mut self.buffer)
        {
            Ok(()) => log::info!("STATUS: {:#x}", self.buffer[0]),
            Err(e) => {
                log::info!("I2C ERROR: {e:?}");
            }
        }

        // Only read if DATA_READY (bit 3) is set
        if self.buffer[0] & 0x08 == 0 {
            log::info!("Data not ready yet");
            return false;
        }

        true
    }

    pub fn get_data(&mut self) -> Option<(u16, u16)> {
        match self
            .i2c
            .blocking_write_read(CCS811_ADDR, &[REG_ALG_RESULT_DATA], &mut self.buffer)
        {
            Ok(()) => {
                let eco2_reading = u16::from_be_bytes([self.buffer[0], self.buffer[1]]);
                let tvoc_reading = u16::from_be_bytes([self.buffer[2], self.buffer[3]]);

                return Some((eco2_reading, tvoc_reading));
            }
            Err(e) => log::info!("ERROR: {e:?}"),
        }

        None
    }
}
