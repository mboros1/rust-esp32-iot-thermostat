use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use esp_idf_hal::delay::FreeRtos;

use esp_idf_hal::gpio::PinDriver;
use esp_idf_hal::onewire::{OWAddress, OWCommand, OWDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, EspWifi};
use esp_idf_sys::{EspError, ESP_ERR_INVALID_ARG};

// TODO: refactor so thermostat logic is in tasks
// TODO: save temperature to a deque like structure to calculate rolling averages
// TODO: think about thermostat temp controls; like compare the delta/rolling average of temp
// calculations, how much to go over/under the set temperature

static RELAY_HIGH: AtomicBool = AtomicBool::new(false);
static TARGET_TEMP: LazyLock<Arc<Mutex<f32>>> = LazyLock::new(|| Arc::new(Mutex::new(22.0)));

struct PID {
    kp: f32,
    ki: f32,
    kd: f32,
    previous_error: f32,
    integral: f32,
}

impl PID {
    fn new(kp: f32, ki: f32, kd: f32) -> Self {
        PID {
            kp,
            ki,
            kd,
            previous_error: 0.0,
            integral: 0.0,
        }
    }

    fn update(&mut self, setpoint: f32, measured: f32, dt: f32) -> f32 {
        let error = setpoint - measured;
        self.integral += error * dt;
        let derivative = (error - self.previous_error) / dt;
        self.previous_error = error;

        // PID output
        self.kp * error + self.ki * self.integral + self.kd * derivative
    }
}


fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Starting Electric Dreams Super AI Crypto Thermostat");

    let peripherals = Peripherals::take().unwrap();

    let mut pid = PID::new(1.0, 0.1, 0.01);
    let dt = 3.0;

    // TODO: implement BLE configuration

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
            return;
        }
        None => {
            log::info!("No device found");
            return;
        }
    };

    let mut relay_high = false;

    loop {
        ds18b20_trigger_temp_conversion(&device, &onewire_bus).unwrap();
        let temp = ds18b20_get_temperature(&device, &onewire_bus).unwrap();
        log::info!("Temperature: {} C, {} F", temp, c_to_f(temp));
        
        /* TODO: implement BLE temp_characteristic
         // Send temperature over BLE
         let temp_str = format!("{:.2}", temp);
         temp_characteristic.set_value(temp_str.as_bytes())?;
         temp_characteristic.notify()?; 
        */

        /* TODO: implement BLE target_characteristic
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

        log::info!("Target Temperature: {} C, {}F", target_temp, c_to_f(target_temp));
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




#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_simulation() {
        // Define the PID parameters
        let kp = 2.0;   // Proportional gain
        let ki = 0.5;   // Integral gain
        let kd = 1.0;   // Derivative gain
        let mut pid = PID::new(kp, ki, kd);

        // Simulation parameters
        let target_temp = 22.0; // Target temperature in Celsius
        let mut current_temp = 18.0; // Starting temperature
        let mut thermostat_on = false; // Initial thermostat state (off)

        // Constants for the temperature simulation
        const HEAT_RATE: f32 = 0.5;  // Temperature increase per iteration when on
        const COOL_RATE: f32 = 0.1;  // Temperature decrease per iteration when off

        // Store temperature history for visualization
        let mut temp_history = vec![];

        // Run the simulation for 100 iterations
        for _ in 0..100 {
            // Simulate temperature dynamics
            if thermostat_on {
                current_temp += HEAT_RATE; // Increase temperature when thermostat is on
            } else {
                current_temp -= COOL_RATE; // Decrease temperature when thermostat is off
            }

            // PID calculation
            let output = pid.update(target_temp, current_temp, 1.0); // Assume dt = 1.0 second

            // Update thermostat state based on PID output
            thermostat_on = output > 0.0;

            // Log current temperature
            temp_history.push(current_temp);

            // Print for debugging
            println!(
                "Temp: {:.2}, Output: {:.2}, Thermostat: {}",
                current_temp,
                output,
                if thermostat_on { "On" } else { "Off" }
            );
        }

        // Check if the temperature stabilizes near the target
        let final_temp = *temp_history.last().unwrap();
        assert!((final_temp - target_temp).abs() < 0.5, "Temperature did not stabilize");
    }
}
