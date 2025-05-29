use std::env;
use std::sync::mpsc;

use p8020::test::{TestNotification, TestState};
use p8020::test_config::builtin::BUILTIN_CONFIGS;
use p8020::{Action, Device, DeviceNotification};

fn print_available_configs() {
    eprintln!("Available protocols:");
    for config in (*BUILTIN_CONFIGS).values() {
        eprintln!("\t{0} ({1})", config.id, config.name);
    }
}

fn main() {
    eprintln!("P8020A test binary (v{})", env!("CARGO_PKG_VERSION"));

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("\nusage: particle-reader <device> [<protocol>]\n");
        print_available_configs();
        return;
    }
    let path = &args[1];
    let protocol_name = if args.len() >= 3 {
        &args[2]
    } else {
        &"osha_legacy".to_string()
    };
    let Some((_, test_config)) = (*BUILTIN_CONFIGS)
        .iter()
        .find(|(id, _)| *id == protocol_name)
    else {
        eprintln!("Protocol {protocol_name} not found.\n");
        print_available_configs();
        return;
    };
    println!("Starting Test, protocol: {0}", test_config.name);

    let (tx_connection_closed, rx_done) = mpsc::channel();
    let tx_request_exit = tx_connection_closed.clone();
    let tx_test_done = tx_connection_closed.clone();

    let device_callback = move |notification: DeviceNotification| {
        match notification {
            DeviceNotification::ConnectionClosed => {
                tx_connection_closed.send(()).unwrap();
            }
            DeviceNotification::DeviceProperties(properties) => {
                // Slight race condition: this might arrive _after_ the first
                // datapoint if the device was already in external control mode.
                eprintln!(
                    "8020(A): #{0} (last serviced: {1}-{2}, runtime since last service: {3})",
                    properties.serial_number,
                    properties.last_service_year,
                    properties.last_service_month,
                    properties.run_time_since_last_service_hours as usize
                );
            }
            DeviceNotification::TestCompleted { .. } => {
                tx_test_done.send(()).unwrap();
            }
            _ => (),
        };
    };

    let test_callback = move |notification: &TestNotification| match notification {
        TestNotification::ExerciseResult {
            exercise,
            fit_factor,
            ..
        } => {
            println!("Exercise {exercise}: FF {fit_factor}")
        }
        TestNotification::StateChange {
            test_state: TestState::StartedExercise { exercise },
        } => {
            eprintln!("Started Exercise {0}", exercise + 1);
        }
        _ => (),
    };

    ctrlc::set_handler(move || {
        tx_request_exit.send(()).unwrap();
    })
    .unwrap();

    match Device::connect_path(path, Some(device_callback)) {
        Ok(device) => {
            // TODO: fix the race condition that requires us to wait prior to
            // starting the test (or else the test gets stuck on the wrong valve state).
            std::thread::sleep(std::time::Duration::from_secs(5));

            device.perform_action(Action::StartTest {
                config: test_config.clone(),
                test_callback: Some(Box::new(test_callback)),
                device_synchroniser: None,
            });
            rx_done.recv().expect("rx_done failed");
        }
        Err(e) => {
            eprintln!("Failed to connect to device: {e}");
        }
    }
}
