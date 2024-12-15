extern crate serialport;
use std::io::BufRead;

// TODO: enumerate devices dynamically
const DEVICE: &str = "/dev/ttyUSB0";

fn send(port: &mut Box<dyn serialport::SerialPort>, msg: &str) {
    if !msg.is_ascii() {
        eprintln!("Unexpected non-ascii msg: {}", msg);
        // TODO: switch to proper error handling.
        std::process::exit(0);
    }

    let mut len_written = port.write(msg.as_bytes()).unwrap();
    len_written += port.write(b"\r").unwrap();
    if len_written != (msg.len() + 1) {
        eprintln!(
            "Expected to write {} bytes, actually wrote {}.",
            msg.len() + 1,
            len_written
        );
        std::process::exit(0);
    }
}

fn main() {
    eprintln!(
        "P8020A reader binary (v{}). (Please note: all I can do is log raw data.)",
        env!("CARGO_PKG_VERSION")
    );

    // See "PortaCount Plus Model 8020 Technical Addendum" for specs.
    // Note: baud is configurable on the devices itself, 1200 is the default.
    let mut port = serialport::new(DEVICE, /* baud_rate */ 1200)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .timeout(core::time::Duration::new(15, 0))
        .open()
        .expect("Unable to open serial port, sorry");

    send(&mut port, "J");

    let reader = std::io::BufReader::new(port);

    for line in reader.lines() {
        let date_time = time::OffsetDateTime::now_utc();
        let format = time::macros::format_description!(
            version = 2,
            "[year]-[month]-[day]T[hour]:[minute]:[second]"
        );
        let formatted_date_time = date_time.format(&format).unwrap();

        // Note: will contain trailing CR (8020A sends CR+LF, BufReader removes the LF).
        // println!("Received: {} @ {}", line.unwrap(), formatted_date_time);
        println!("{},{}", formatted_date_time, line.unwrap().trim());
    }

    // TODO: check N95 companion.
    // TODO: release from external control on exit.
}
