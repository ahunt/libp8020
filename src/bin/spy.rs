extern crate serialport;
use std::io::BufRead;

// TODO: enumerate devices dynamically
const DEVICE: &str = "/dev/ttyUSB0";

fn main() {
    eprintln!(
        "P8020A spy (v{}). (This binary simply dumps your Portacount's serial output, because I'm too lazy to remember the appropriate commands.)",
        env!("CARGO_PKG_VERSION")
    );

    // See "PortaCount Plus Model 8020 Technical Addendum" for specs.
    // Note: baud is configurable on the devices itself, 1200 is the default.
    let port = serialport::new(DEVICE, /* baud_rate */ 1200)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        // Very long timeout, because the portacount might send nothing when not in use
        .timeout(core::time::Duration::from_secs(60 * 60 * 24))
        .open()
        .expect("Unable to open serial port, sorry");

    let reader = std::io::BufReader::new(port);

    for line in reader.lines() {
        println!("{}", line.unwrap().trim());
    }
}
