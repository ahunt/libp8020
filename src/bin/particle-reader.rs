use std::env;
use std::sync::mpsc;

use p8020::{Device, DeviceNotification};

fn main() {
    eprintln!("P8020A reader binary (v{})", env!("CARGO_PKG_VERSION"));

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("\nusage: particle-reader <device>");
        return;
    }
    let path = &args[1];

    let (tx_connection_closed, rx_done) = mpsc::channel();
    let tx_request_exit = tx_connection_closed.clone();

    let callback = move |notification: DeviceNotification| {
        match notification {
            DeviceNotification::Sample { particle_conc } => {
                let date_time = time::OffsetDateTime::now_utc();
                let format = time::macros::format_description!(
                    version = 2,
                    "[year]-[month]-[day]T[hour]:[minute]:[second]"
                );
                let formatted_date_time = date_time.format(&format).unwrap();
                println!("{},{}", formatted_date_time, particle_conc);
            }
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
            _ => (),
        };
    };

    ctrlc::set_handler(move || {
        tx_request_exit.send(()).unwrap();
    })
    .unwrap();

    match Device::connect_path(path, Some(callback)) {
        // _device must be kept alive to keep the connection alive.
        Ok(_device) => {
            rx_done.recv().expect("rx_done failed");
        }
        Err(e) => {
            eprintln!("Failed to connect to device: {e}");
        }
    }
}
