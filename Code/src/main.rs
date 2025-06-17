#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_net::{Config as NetConfig, Ipv4Address, Ipv4Cidr, StackResources, tcp::TcpSocket};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{Config as I2cConfig, I2c, InterruptHandler as I2CInterruptHandler};
use embassy_rp::peripherals::{I2C0, I2C1, USB};
use embassy_rp::spi::{Async, Config as SpiConfig, Spi};
use embassy_rp::usb::InterruptHandler;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_1::delay::DelayNs;
use heapless::String;
use lcd1602_diver::{self as display_word, Direction, Display};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;

use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    I2C0_IRQ => I2CInterruptHandler<I2C0>;
    I2C1_IRQ => I2CInterruptHandler<I2C1>;
    USBCTRL_IRQ => InterruptHandler<USB>;
});

static RESOURCES: StaticCell<StackResources<SOCK>> = StaticCell::<StackResources<SOCK>>::new();
static SHARED_CHANNEL: Channel<ThreadModeRawMutex, (u8, u8, u8), 6> = Channel::new();

const SOCK: usize = 4;

const MAX_TRIES: u8 = 6;

const ROW_0: u8 = 0x01;
const LEDS_ON_5: u8 = 0xF9;
const SHUTDOWN: u8 = 0x0C;
const INTENSITY: u8 = 0x0A;
const SCAN_LIMIT: u8 = 0x0B;
const DECODE_MODE: u8 = 0x09;
const DISPLAY_TEST: u8 = 0x0F;

const I2C_DISPLAY_REG: u8 = 0x27;
const SECOND_ROW_DISPLAY: u8 = 40;
const TIME_BETWEEN_MESSAGES: u32 = 2000;

const CLK_FREQUENCY: u32 = 1_000_000;

async fn max7219_write(
    spi: &mut Spi<'_, embassy_rp::peripherals::SPI0, Async>,
    cs: &mut Output<'_>,
    reg: u8,
    data: u8,
) {
    cs.set_low();
    let _ = spi.write(&[reg, data]).await;
    cs.set_high();
}

// Task destinated to cycling between the bits to make the blinking effect
// for present and correct letters
#[embassy_executor::task]
async fn blink_task(
    mut spi: Spi<'static, embassy_rp::peripherals::SPI0, Async>,
    mut cs: Output<'static>,
) {
    let mut correct_leds = [0u8; 6];
    let mut present_leds = [0u8; 6];

    loop {
        loop {
            match SHARED_CHANNEL.try_receive() {
                Ok((size, correct_letter, present_letter)) => {
                    correct_leds[size as usize] = correct_letter;
                    present_leds[size as usize] = present_letter;
                }
                Err(_) => break,
            }
        }

        for row in 0..6 {
            max7219_write(&mut spi, &mut cs, ROW_0 + row, correct_leds[row as usize]).await;
        }
        Timer::after(Duration::from_millis(500)).await;

        for row in 0..6 {
            let value = correct_leds[row] | present_leds[row];
            max7219_write(&mut spi, &mut cs, ROW_0 + row as u8, value).await;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

// Function to process the input word and the leds who need to be turned on
async fn check_word(actual_word: &str, target_word: &str, tries: u8) -> bool {
    if actual_word == target_word {
        SHARED_CHANNEL.send((tries, LEDS_ON_5, 0)).await;
        return true;
    }

    let mut correct_letters = [false; 5];
    let mut present_letters = [false; 5];

    for i in 0..5 {
        if actual_word.chars().nth(i) == target_word.chars().nth(i) {
            correct_letters[i] = true;
        }
    }

    for i in 0..5 {
        for j in 0..5 {
            if actual_word.chars().nth(i) == target_word.chars().nth(j) && !correct_letters[j] {
                present_letters[i] = true;
            }
        }
    }

    let mut result_led_correct_letter = 0b00000001;
    let mut result_led_present_letter = 0b00000001;
    for i in 0..5 {
        if correct_letters[i] {
            result_led_correct_letter |= 1 << (7 - i);
        } else if present_letters[i] {
            result_led_present_letter |= 1 << (7 - i);
        }
    }

    SHARED_CHANNEL
        .send((tries, result_led_correct_letter, result_led_present_letter))
        .await;

    return false;
}

// Print a message for a limited time on the LCD display
fn print_message(
    display: &mut lcd1602_diver::LCD1602<
        lcd1602_diver::data_bus::I2CBus<I2c<'_, I2C0, embassy_rp::i2c::Async>>,
    >,
    message: &str,
    delay: &mut Delay,
) {
    let _ = display.set_cursor_pos(SECOND_ROW_DISPLAY, delay);
    let _ = display.write_str(message, delay);

    let _ = delay.delay_ms(TIME_BETWEEN_MESSAGES);

    let _ = display.set_cursor_pos(SECOND_ROW_DISPLAY, delay);
    let _ = display.write_str("                 ", delay);
    let _ = display.set_cursor_pos(SECOND_ROW_DISPLAY, delay);
    let _ = display.write_str("     ", delay);
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());

    // Configure pins for I2C and SPI
    let sda = peripherals.PIN_21;
    let scl = peripherals.PIN_20;
    let clk = peripherals.PIN_18;
    let mosi = peripherals.PIN_19;

    let sda_2 = peripherals.PIN_15;
    let scl_2 = peripherals.PIN_14;

    // Congigure SPI for the leds matrix
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = CLK_FREQUENCY;
    let mut spi = Spi::new_txonly(peripherals.SPI0, clk, mosi, peripherals.DMA_CH0, spi_config);
    let mut cs = Output::new(peripherals.PIN_5, Level::High);

    let mut delay = Delay;
    let dis = Display::On;

    // Configure I2C for the LCD display (main display)
    let i2c = I2c::new_async(peripherals.I2C0, sda, scl, Irqs, I2cConfig::default());
    let lcd = display_word::LCD1602::new_i2c(i2c, I2C_DISPLAY_REG, &mut delay);
    let mut display = lcd.unwrap();

    // Configure I2C for the LCD display (history display)
    let i2c_2 = I2c::new_async(peripherals.I2C1, sda_2, scl_2, Irqs, I2cConfig::default());
    let lcd_2 = display_word::LCD1602::new_i2c(i2c_2, I2C_DISPLAY_REG, &mut delay);
    let mut display_history = lcd_2.unwrap();

    // Initialize the displays
    let _ = display_history.reset(&mut delay);
    let _ = display_history.set_display(dis, &mut delay);
    let _ = display_history.set_cursor_pos(0, &mut delay);

    let _ = display.reset(&mut delay);
    let _ = display.set_display(dis, &mut delay);
    let _ = display.set_cursor_pos(2, &mut delay);
    let _ = display.write_str("Pico-Wordle", &mut delay);

    let _ = display.set_cursor_pos(SECOND_ROW_DISPLAY, &mut delay);
    let _ = display.write_str("     ", &mut delay);

    // Initialize MAX7219
    max7219_write(&mut spi, &mut cs, SHUTDOWN, 0x00).await;
    max7219_write(&mut spi, &mut cs, DECODE_MODE, 0x00).await;
    max7219_write(&mut spi, &mut cs, SCAN_LIMIT, 0x07).await;
    max7219_write(&mut spi, &mut cs, INTENSITY, 0x08).await;
    max7219_write(&mut spi, &mut cs, DISPLAY_TEST, 0x00).await;
    max7219_write(&mut spi, &mut cs, SHUTDOWN, 0x01).await;

    // Clear display
    for i in 0..8 {
        max7219_write(&mut spi, &mut cs, ROW_0 + i, 0x00).await;
    }

    spawner.spawn(blink_task(spi, cs)).expect("Task faild");

    let (net_device, mut control) = embassy_lab_utils::init_wifi!(&spawner, peripherals).await;

    control.start_ap_wpa2("Pico_Wordle", "toporas12", 3).await;

    // Configure static IP
    let config = NetConfig::ipv4_static(embassy_net::StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(169, 254, 1, 1), 16),
        dns_servers: heapless::Vec::new(),
        gateway: None,
    });
    let stack = embassy_lab_utils::init_network_stack(&spawner, net_device, &RESOURCES, config);

    // Set up TCP server
    let mut rx_buffer = [0; 8];
    let mut tx_buffer = [0; 8];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(None);

    // Select the random word from the list
    let words_list = [
        "actor", "adult", "album", "carne", "casca", "cizma", "dieta", "dolar", "ecran", "epava",
        "flota", "forta", "munte", "moara", "panta", "nisip", "soare", "sursa", "tinta", "torta",
        "vapor", "vesta", "zebra", "bivol", "ceara", "dulce", "epoca", "harta", "lalea", "marea",
        "nervi", "oaste", "radio", "tigru", "umbra",
    ];

    let mut rng = SmallRng::seed_from_u64(embassy_time::Instant::now().as_ticks());
    let target_word = words_list.choose(&mut rng).expect("List is empty");
    info!("Selected word is: {}", target_word);

    info!("Listening on TCP:6000...");
    loop {
        if let Err(e) = socket.accept(6000).await {
            warn!("accept error: {:?}", e);
            continue;
        }

        info!("Received connection from {:?}", socket.remote_endpoint());

        let mut buf = [0; 8];
        let mut actual_word: String<5> = String::new();
        let mut tries = 0;

        loop {
            match socket.read(&mut buf).await {
                Ok(0) => {
                    info!("Connection closed");
                    break;
                }
                Ok(n) => {
                    let data = &buf[..n];
                    let letter = data[0] as char;

                    if actual_word.len() >= 5 && letter != 'E' && letter != 'D' {
                        info!("Too many letters");

                        print_message(&mut display, "Too many letters", &mut delay);
                        let _ = display.write_str(&actual_word, &mut delay);
                        continue;
                    }

                    match letter {
                        'E' => {
                            info!("Enter pressed");

                            if actual_word.len() == 5 {
                                let result = check_word(&actual_word, target_word, tries).await;
                                if result {
                                    let _ = display.clear(&mut delay);
                                    let _ = display.set_cursor_pos(4, &mut delay);
                                    let _ = display.write_str("You win!", &mut delay);
                                    break;
                                } else {
                                    info!("Word not found");
                                    print_message(&mut display, "Word not found", &mut delay);

                                    // Write the word to the history display
                                    if tries < 2 {
                                        let _ = display_history.write_str(&actual_word, &mut delay);
                                        let _ = display_history.write_str(" ", &mut delay);
                                    } else if tries == 2 {
                                        let first_part = &actual_word[0..3];
                                        let _ = display_history.write_str(first_part, &mut delay);
                                        let _ = display_history
                                            .set_cursor_pos(SECOND_ROW_DISPLAY, &mut delay);

                                        let _ = display_history
                                            .write_str(&actual_word[3..], &mut delay);
                                        let _ = display_history.write_str(" ", &mut delay);
                                    } else {
                                        let _ = display_history.write_str(&actual_word, &mut delay);
                                        let _ = display_history.write_str(" ", &mut delay);
                                    }

                                    actual_word.clear();
                                    tries += 1;
                                    if tries == MAX_TRIES {
                                        let _ = display.clear(&mut delay);
                                        let _ = display.set_cursor_pos(4, &mut delay);
                                        let _ = display.write_str("You lose!", &mut delay);
                                        let _ =
                                            display.set_cursor_pos(SECOND_ROW_DISPLAY, &mut delay);
                                        let _ = display.write_str(target_word, &mut delay);
                                        break;
                                    }
                                }
                            } else {
                                info!("Word too short");
                                print_message(&mut display, "Word too short", &mut delay);
                                let _ = display.write_str(&actual_word, &mut delay);
                            }
                        }

                        'D' => {
                            info!("Delete pressed");
                            if actual_word.len() == 0 {
                                continue;
                            }

                            let _ = display.shift_cursor(Direction::Left, &mut delay);
                            let _ = display.write_char(' ', &mut delay);
                            let _ = display.shift_cursor(Direction::Left, &mut delay);
                            let _ = actual_word.pop();
                        }

                        _ => {
                            let _ = display.write_char(letter, &mut delay);
                            let _ = actual_word.push(letter);
                        }
                    }
                    info!("Actual word: {}", actual_word);
                }
                Err(e) => {
                    warn!("Read error: {:?}", e);
                    break;
                }
            }
        }

        socket.close();
        break;
    }

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
