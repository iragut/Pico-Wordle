#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::i2c::{Config as I2cConfig, I2c, InterruptHandler as I2CInterruptHandler};
use embassy_rp::peripherals::I2C0;
use embassy_time::Delay;
use lcd1602_diver::{self as display_word, Display};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    I2C0_IRQ => I2CInterruptHandler<I2C0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // Get a handle to the RP's peripherals.
    let peripherals = embassy_rp::init(Default::default());

    let sda = peripherals.PIN_21;
    let scl = peripherals.PIN_20;

    let i2c = I2c::new_async(peripherals.I2C0, sda, scl, Irqs, I2cConfig::default());

    let mut delay = Delay;

    let lcd = display_word::LCD1602::new_i2c(i2c, 0x27, &mut delay);

    let mut dis = lcd.unwrap();

    let _ = dis.reset(&mut delay);

    let dispplay = Display::On;

    let _ = dis.set_display(dispplay, &mut delay);
    let _ = dis.set_cursor_pos(0, &mut delay);
    let _ = dis.write_str("Hello ", &mut delay);

    loop {
        // Wait for 1 second
        embassy_time::Timer::after(embassy_time::Duration::from_millis(1000)).await;
        // Print a message to the console
        info!("Hello, world!");
    }
}
