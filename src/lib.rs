pub struct PID {
    kp: f32,
    ki: f32,
    kd: f32,
    previous_error: f32,
    integral: f32,

    integral_min: f32,
    integral_max: f32,
    output_min: f32,
    output_max: f32,
}

impl PID {
    pub fn new(kp: f32, ki: f32, kd: f32) -> Self {
        PID {
            kp,
            ki,
            kd,
            previous_error: 0.0,
            integral: 0.0,
            integral_min: -1000.0,
            integral_max: 1000.0,
            output_min: -1.0,
            output_max: 1.0,
        }
    }

    pub fn update(&mut self, setpoint: f32, measured: f32, dt: f32) -> f32 {
        let error = setpoint - measured;

        // Calculate potential new integral
        self.integral += error * dt;

        // Apply anti-windup protection before updating
        self.integral = self.integral.clamp(self.integral_min, self.integral_max);

        let derivative = -(error - self.previous_error) / dt;
        self.previous_error = error;

        // Calculate raw output
        let output = self.kp * error + self.ki * self.integral + self.kd * derivative;

        // Final output clamping
        output.clamp(self.output_min, self.output_max)
    }

    pub fn with_integral_limits(&mut self, min: f32, max: f32) -> &mut PID {
        self.integral_min = min;
        self.integral_max = max;
        self
    }

    pub fn with_output_limits(&mut self, min: f32, max: f32) -> &mut PID {
        self.output_min = min;
        self.output_max = max;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_simulation() {
        // Define the PID parameters
        let kp = 2.0; // Proportional gain
        let ki = 0.5; // Integral gain
        let kd = 1.0; // Derivative gain
        let mut pid = PID::new(kp, ki, kd);

        // Simulation parameters
        let target_temp = 22.0; // Target temperature in Celsius
        let mut current_temp = 18.0; // Starting temperature
        let mut thermostat_on = false; // Initial thermostat state (off)

        // Constants for the temperature simulation
        const HEAT_RATE: f32 = 0.5; // Temperature increase per iteration when on
        const COOL_RATE: f32 = 0.1; // Temperature decrease per iteration when off

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
        assert!(
            (final_temp - target_temp).abs() < 0.5,
            "Temperature did not stabilize"
        );
    }
}
