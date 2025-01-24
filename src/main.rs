use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use esp32_nimble::utilities::BleUuid;
use esp32_nimble::{uuid128, BLEAdvertisementData, OnWriteArgs};
use esp32_nimble::{BLEAdvertising, NimbleProperties};
use esp_idf_hal::delay::FreeRtos;

use esp_idf_hal::gpio::PinDriver;
use esp_idf_hal::onewire::{OWAddress, OWCommand, OWDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;

// TODO: run the PID simulation locally; currently it requires a connected esp32c3 to run
// TODO: maybe also add to run simulated esp32 in qemu

use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, EspWifi};
use esp_idf_sys::{EspError, ESP_ERR_INVALID_ARG};
use rust_esp32_iot_thermostat::PID;

// TODO: refactor so thermostat logic is in tasks
// TODO: save temperature to a deque like structure to calculate rolling averages
// TODO: think about thermostat temp controls; like compare the delta/rolling average of temp
// calculations, how much to go over/under the set temperature

const THERMOSTAT_SERVICE_UUID: BleUuid = uuid128!("6e400001-b5a3-f393-e0a9-e50e24dcca9e");
const TEMPERATURE_CHAR_UUID: BleUuid = uuid128!("6e400002-b5a3-f393-e0a9-e50e24dcca9e");
const SETPOINT_CHAR_UUID: BleUuid = uuid128!("6e400003-b5a3-f393-e0a9-e50e24dcca9e");

struct ThermostatBLE {
    temperature_char: Arc<esp32_nimble::utilities::mutex::Mutex<esp32_nimble::BLECharacteristic>>,
    target_temperature_char:
        Arc<esp32_nimble::utilities::mutex::Mutex<esp32_nimble::BLECharacteristic>>,
    target_temperature: Arc<Mutex<f32>>,
}

impl ThermostatBLE {
    fn new() -> anyhow::Result<Self> {
        let ble_device = esp32_nimble::BLEDevice::take();

        let server = ble_device.get_server();

        // Service creation with BleUuid
        let service = server.create_service(THERMOSTAT_SERVICE_UUID);

        // Create target temperature storage
        let target_temperature = Arc::new(Mutex::new(20.0)); // Default value

        // Clone for callback
        let target_temp_cb = Arc::clone(&target_temperature);

        // Characteristic creation with BleUuid
        let temperature_char = service.lock().create_characteristic(
            TEMPERATURE_CHAR_UUID,
            NimbleProperties::READ | NimbleProperties::NOTIFY,
        );

        let target_temperature_char = {
            let char = service.lock().create_characteristic(
                SETPOINT_CHAR_UUID,
                NimbleProperties::READ | NimbleProperties::WRITE,
            );

            char.lock().on_write(move |args: &mut OnWriteArgs| {
                match parse_temperature(args.recv_data()) {
                    Ok(temp) => {
                        *target_temp_cb.lock().unwrap() = temp;
                        log::info!("New target temp: {}", temp);
                    }
                    Err(e) => {
                        log::error!("Invalid temperature data: {}", e);
                        args.reject_with_error_code(0x80); // Invalid Attribute Value error
                    }
                }
            });

            char
        };

        let advertising = ble_device.get_advertising();
        // Configure advertising after service setup

        service
            .lock()
            .create_characteristic(
                uuid128!("00002a01-0000-1000-8000-00805f9b34fb"), // Appearance ID
                NimbleProperties::READ,
            )
            .lock()
            .set_value(&[0x00, 0x00]); // Generic Unknown :cite[6]

        advertising.lock().set_data(
            BLEAdvertisementData::new()
                .name("Electric Dreams Super AI Crypto Thermostat")
                .add_service_uuid(THERMOSTAT_SERVICE_UUID),
        )?;

        // Start advertising indefinitely
        advertising.lock().start()?;
        log::info!("BLE advertising started");

        Ok(Self {
            temperature_char,
            target_temperature_char,
            target_temperature,
        })
    }

    fn update_temperature(&self, value: f32) {
        let mut char = self.temperature_char.lock();
        char.set_value(&value.to_le_bytes());
        char.notify();
    }
}

fn parse_temperature(data: &[u8]) -> anyhow::Result<f32> {
    let bytes: [u8; 4] = data
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid temperature data length"))?;
    Ok(f32::from_le_bytes(bytes))
}

static RELAY_HIGH: AtomicBool = AtomicBool::new(false);
static TARGET_TEMP: LazyLock<Arc<Mutex<f32>>> = LazyLock::new(|| Arc::new(Mutex::new(22.0)));

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Starting Electric Dreams Super AI Crypto Thermostat");

    let ble = ThermostatBLE::new().unwrap();

    let peripherals = Peripherals::take().unwrap();

    let mut pid = PID::new(1.0, 0.1, 0.01);
    let dt = 3.0;

    let pin = peripherals.pins.gpio5;
    let pin4 = peripherals.pins.gpio4;

    let mut relay_input = PinDriver::output(pin4).unwrap();

    let channel = peripherals.rmt.channel0;

    let mut onewire_bus: OWDriver = OWDriver::new(pin, channel).unwrap();

    let device = match onewire_bus.search().unwrap().next() {
        Some(Ok(dev)) => {
            log::info!(
                "Found device: {:?}, family code = {}",
                dev,
                dev.family_code()
            );
            dev
        }
        Some(Err(err)) => {
            log::error!("Error occured searching for device: {:?}", err);
            return Ok(());
        }
        None => {
            log::info!("No device found");
            return Ok(());
        }
    };

    loop {
        ds18b20_trigger_temp_conversion(&device, &onewire_bus).unwrap();
        let temp = ds18b20_get_temperature(&device, &onewire_bus).unwrap();
        log::info!("Temperature: {} C, {} F", temp, c_to_f(temp));

        // Send temperature over BLE
        ble.update_temperature(temp);

        /*
        // Update the target temperature if a new value is written via BLE
        if let Ok(target_temp_value) = target_characteristic.get_value() {
            if let Ok(target_temp_str) = std::str::from_utf8(&target_temp_value) {
                if let Ok(new_target) = target_temp_str.parse::<f32>() {
                    let mut target_temp = TARGET_TEMP.lock().unwrap(); // Lock mutex
                    *target_temp = new_target; // Update value
                    log::info!("Updated target temperature to: {} C", new_target);
                }
            }
        }
        */

        // Calculate PID output
        let target_temp = *TARGET_TEMP.lock().unwrap(); // Access the value safely
        let output = pid.update(target_temp, temp, dt);
        // Determine relay state based on PID output
        if output > 0.0 {
            RELAY_HIGH.store(true, Ordering::SeqCst);
            relay_input.set_high().unwrap(); // Turn on the heater
            log::info!("Relay input high! (Heater ON)");
        } else {
            RELAY_HIGH.store(false, Ordering::SeqCst);
            relay_input.set_low().unwrap(); // Turn off the heater
            log::info!("Relay input low! (Heater OFF)");
        }

        let error = target_temp - temp;

        log::info!(
            "Target Temperature: {} C, {}F",
            target_temp,
            c_to_f(target_temp)
        );
        log::info!("Current error: {}, dt: {}", error, dt);

        FreeRtos::delay_ms(3000);
    }
}

pub fn c_to_f(celsius: f32) -> f32 {
    celsius * 9.0 / 5.0 + 32.0
}

fn ds18b20_send_command(addr: &OWAddress, bus: &OWDriver, cmd: u8) -> Result<(), EspError> {
    let mut buf = [0; 10];
    buf[0] = OWCommand::MatchRom as _;
    let addr = addr.address().to_le_bytes();
    buf[1..9].copy_from_slice(&addr);
    buf[9] = cmd;

    bus.write(&buf)
}

#[allow(dead_code)]
#[repr(u8)]
enum Ds18b20Command {
    ConvertTemp = 0x44,
    WriteScratch = 0x4E,
    ReadScratch = 0xBE,
}

fn ds18b20_trigger_temp_conversion(addr: &OWAddress, bus: &OWDriver) -> Result<(), EspError> {
    // reset bus and check if the ds18b20 is present
    bus.reset()?;

    ds18b20_send_command(addr, bus, Ds18b20Command::ConvertTemp as u8)?;

    // delay proper time for temp conversion,
    // assume max resolution (12-bits)
    std::thread::sleep(Duration::from_millis(800));

    Ok(())
}

fn ds18b20_get_temperature(addr: &OWAddress, bus: &OWDriver) -> Result<f32, EspError> {
    bus.reset()?;

    ds18b20_send_command(addr, bus, Ds18b20Command::ReadScratch as u8)?;

    let mut buf = [0u8; 9];
    bus.read(&mut buf)?;
    let lsb = buf[0];
    let msb = buf[1];

    let temp_raw: u16 = (u16::from(msb) << 8) | u16::from(lsb);

    Ok(f32::from(temp_raw) / 16.0)
}

pub fn wifi(
    ssid: &str,
    pass: &str,
    modem: impl esp_idf_svc::hal::peripheral::Peripheral<P = esp_idf_svc::hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> Result<Box<EspWifi<'static>>, EspError> {
    let mut auth_method = AuthMethod::WPA2Personal;
    if ssid.is_empty() {
        log::error!("Missing WiFi name");
        return Err(EspError::from(ESP_ERR_INVALID_ARG).unwrap());
    }
    if pass.is_empty() {
        auth_method = AuthMethod::None;
        log::info!("Wifi password is empty");
    }
    let nvs = esp_idf_svc::nvs::EspDefaultNvsPartition::take().ok();
    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), nvs)?;

    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.set_configuration(&esp_idf_svc::wifi::Configuration::Client(
        ClientConfiguration::default(),
    ))?;

    log::info!("Starting wifi...");

    wifi.start()?;

    log::info!("Scanning...");

    let ap_infos = wifi.scan()?;

    let ours = ap_infos.into_iter().find(|a| a.ssid == ssid);

    let channel = if let Some(ours) = ours {
        log::info!(
            "Found configured access point {} on channel {}",
            ssid,
            ours.channel
        );
        Some(ours.channel)
    } else {
        log::info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            ssid
        );
        None
    };

    wifi.set_configuration(&esp_idf_svc::wifi::Configuration::Client(
        ClientConfiguration {
            ssid: ssid
                .try_into()
                .expect("Could not parse the given SSID into WiFi config"),
            password: pass
                .try_into()
                .expect("Could not parse the given password into WiFi config"),
            channel,
            auth_method,
            ..Default::default()
        },
    ))?;

    log::info!("Connecting wifi...");

    wifi.connect()?;

    log::info!("Waiting for DHCP lease...");

    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    log::info!("Wifi DHCP info: {:?}", ip_info);

    Ok(Box::new(esp_wifi))
}
