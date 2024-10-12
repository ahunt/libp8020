extern crate serialport;
use clap::Parser;
use std::io::BufRead;
use std::str::FromStr;

// TODO: enumerate devices dynamically
const DEVICE: &str = "/dev/ttyUSB0";

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Number of exercises
    #[arg(long, default_value_t = 8)]
    exercises: usize,

    #[arg(long, default_value_t = 4)]
    ambient_purge_time: usize,

    #[arg(long, default_value_t = 5)]
    ambient_sample_time: usize,

    #[arg(long, default_value_t = 11)]
    specimen_purge_time: usize,

    #[arg(long, default_value_t = 40)]
    specimen_sample_time: usize,
}

#[derive(Clone)]
struct Exercise {
    ambient_purges_done: usize,
    ambient_samples: std::vec::Vec<f64>,
    specimen_switch_received: bool,
    specimen_purges_done: usize,
    specimen_samples: std::vec::Vec<f64>,
}

impl Exercise {
    fn new(args: &Args) -> Exercise {
        Exercise {
            ambient_purges_done: 0,
            ambient_samples: Vec::with_capacity(args.ambient_sample_time),
            specimen_switch_received: false,
            specimen_purges_done: 0,
            specimen_samples: Vec::with_capacity(args.specimen_sample_time),
        }
    }
}

fn send(port: &mut Box<dyn serialport::SerialPort>, msg: &str) {
    if !msg.is_ascii() {
        eprintln!("Unexpected non-ascii msg: {}", msg);
        // TODO: switch to proper error handling.
        std::process::exit(0);
    }

    let mut len_written = port.write(msg.as_bytes()).unwrap();
    len_written += port.write(&[b'\r']).unwrap();
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
    let args = Args::parse();
    eprintln!(
        "8020A tester (v{}).\nPerforming {} exercise(s) ({}s/{}s/{}s/{}s)\n\n",
        env!("CARGO_PKG_VERSION"),
        args.exercises,
        args.ambient_purge_time,
        args.ambient_sample_time,
        args.specimen_purge_time,
        args.specimen_sample_time
    );

    // See "PortaCount Plus Model 8020 Technical Addendum" for specs.
    // Note: baud is configurable on the devices itself, 1200 is the default.
    let mut port = serialport::new(DEVICE, /* baud_rate */ 1200)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::Hardware)
        .timeout(core::time::Duration::new(15, 0))
        .open()
        .expect("Unable to open serial port, sorry");

    let mut reader = std::io::BufReader::new(port.try_clone().unwrap());

    // TODO: do some probing first to determine whether the Portacount is
    // already in external control mode etc.
    send(&mut port, "J"); // Invoke External Control

    // Either flow control is broken, or my adapter is broken, or the 8020A is
    // too slow to do flow control right. A 1s sleep after every outgoing
    // message seems to work.
    // TODO: do more testing to verify which (if any) of the above is true.
    std::thread::sleep(std::time::Duration::from_secs(1));
    send(&mut port, "VN"); // Switch valve on

    // Additional exercise is used for the final ambient samples (specimen samples are left empty).
    let exercises = &mut vec![Exercise::new(&args); args.exercises + 1].into_boxed_slice();
    let mut current_exercise = 0;

    // Get rid of any buffered junk - this is possible if the device was already
    // in external control mode. And skip straight to where we switched to
    // ambient sampling.
    for line in (&mut reader).lines() {
        if line.unwrap().trim() == "VN" {
            break;
        }
    }

    send(&mut port, "B40"); // Beep

    for line in reader.lines() {
        let contents = line.unwrap();
        // BufReader removes the trailing <LR>, we need to remove the remaining <CR>.
        let message = contents.trim();
        let current = &mut exercises[current_exercise];
        match message {
            // Docs claim this is "VO", I suspect there was a typo (or the firmware was changed/fixed - the Portacount replies VN to VN, so it should reply VF to VF too?
            "VF" => {
                eprintln!(
                    "Received VF (switched to specimen successfully) after {} purges, {} samples",
                    current.ambient_purges_done,
                    current.ambient_samples.len()
                );
                current.specimen_switch_received = true;
                // Final (i.e. additional) exercise is used only for ambient sample storage.
                if current_exercise == args.exercises + 1 {
                    break;
                }
                continue;
            }
            "VN" => {
                eprintln!(
                    "Received VN (switched to ambient successfully) after {} purges, {} samples",
                    current.specimen_purges_done,
                    current.specimen_samples.len()
                );
                current_exercise += 1;
                // Print after to increment ensure 1-indexed output.
                eprintln!(
                    "Exercise {} done, ambient = {}, specimen = {}",
                    current_exercise,
                    current.ambient_samples.iter().sum::<f64>()
                        / (current.ambient_samples.len() as f64),
                    current.specimen_samples.iter().sum::<f64>()
                        / (current.specimen_samples.len() as f64)
                );
                if current_exercise == args.exercises {
                    break;
                }
                continue;
            }
            ref m if m.starts_with("B") => {
                // Ignore - the Portacount mirrors these.
                continue;
            }
            _ => (),
        }

        let value = match f64::from_str(message) {
            Ok(res) => res,
            Err(_) => {
                eprintln!("Unexpected message received: {}", message);
                continue;
            }
        };

        if current.ambient_purges_done < args.ambient_purge_time {
            current.ambient_purges_done += 1;
        } else if current.ambient_samples.len() < args.ambient_sample_time {
            current.ambient_samples.push(value);
            if current.ambient_samples.len() == args.ambient_sample_time {
                send(&mut port, "VF"); // Switch valve off
            }
        } else if !current.specimen_switch_received {
            eprintln!("Received (unexpected) ambient sample after requesting valve switch. That's fine, it just means something was slow.");
        } else if current.specimen_purges_done < args.specimen_purge_time {
            current.specimen_purges_done += 1;
        } else if current.specimen_samples.len() < args.specimen_sample_time {
            current.specimen_samples.push(value);
            if current.specimen_samples.len() == args.specimen_sample_time {
                send(&mut port, "VN"); // Switch valve on
                std::thread::sleep(std::time::Duration::from_secs(1));
                send(&mut port, "B05"); // Beep
            }
        } else {
            eprintln!("Received (unexpected) specimen sample after requesting valve switch. That's fine, it just means something was slow.");
        }
    }

    send(&mut port, "G"); // Release from external control

    for i in 0..args.exercises {
        let ambient_avg = (exercises[i].ambient_samples.iter().sum::<f64>()
            + exercises[i + 1].ambient_samples.iter().sum::<f64>())
            / ((exercises[i].ambient_samples.len() + exercises[i + 1].ambient_samples.len())
                as f64);
        let specimen_avg = exercises[i].specimen_samples.iter().sum::<f64>()
            / (exercises[i].specimen_samples.len() as f64);
        let fit_factor = ambient_avg / specimen_avg;
        // TODO: 8020A only appears to print decimal for FF < (maybe) 10, should
        // we do the same here?
        println!("Exercise {}: FF {:.1}", i, fit_factor);
    }
    // TODO: print avg FF.
}
