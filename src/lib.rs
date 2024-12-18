extern crate libc;
extern crate serialport;

mod ffi;
pub mod protocol;
mod test;
pub mod test_config;

use serialport::SerialPortInfo;
use std::io::BufRead;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use protocol::{Command, Message, SettingMessage};
use test::{StepOutcome, Test};

enum ValveState {
    Specimen,
    AwaitingAmbient,
    Ambient,
    AwaitingSpecimen,
}

pub enum DeviceNotification {
    /// Sample indicates a fresh reading from the PC. It is safe to assume
    /// that it was delivered 1s (plus/minus the 8020's internal delays) after
    /// the previous RawReading. This is simply the latest sample, no more,
    /// no less - i.e. it might be part of the ambient or specimen purge,
    /// or from the actually sampling period.
    // TODO: check specs for what the actual allowed range is.
    Sample {
        particle_conc: f64,
    },
    TestStarted,
    TestCompleted {
        fit_factors: Vec<f64>,
    },
    TestCancelled,
    ConnectionClosed,
    DeviceProperties {
        serial_number: String,
        run_time_since_last_service_hours: f64,
        last_service_month: u8,
        last_service_year: u16,
    },
}

pub enum Action {
    StartTest {
        config: test_config::TestConfig,
        test_callback: test::TestCallback,
    },
    CancelTest,
}

pub struct Device {
    tx_action: Sender<Action>,
}

impl Device {
    // TODO: add proper error handling (once I've figured out what an
    // appropriate approach is in conjunction with FFI)
    // TODO: switch to a builder pattern for params such as baud rate.
    // Hopefully no one is using other baud rates, but it'd be interesting to
    // experiment regardless.
    pub fn connect(
        port_info: SerialPortInfo,
        device_callback: Option<impl Fn(&DeviceNotification) + 'static + std::marker::Send>,
    ) -> serialport::Result<Device> {
        Device::connect_path(port_info.port_name, device_callback)
    }

    pub fn connect_path(
        path: String,
        // device_callback: Option<fn(&DeviceNotification)>,
        device_callback: Option<impl Fn(&DeviceNotification) + 'static + std::marker::Send>,
    ) -> serialport::Result<Device> {
        // See "PortaCount Plus Model 8020 Technical Addendum" for specs.
        // Note: baud is configurable on the devices itself, 1200 is the default.
        let port = serialport::new(path, /* baud_rate */ 1200)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::Hardware)
            // The timeout is relevant for receiver_thread's behaviour (below).
            .timeout(core::time::Duration::from_millis(100))
            .open()?;

        // Cloning here is a bit ugly - it's necessary because we want to split reads
        // and writes, and Serialport implements both in the same object. Read and
        // writes are mutating, hence an Arc is insufficient. A (rust) Mutex also
        // doesn't work because reads and writes need to be independent. Writing
        // some kind of custom wrapper (possibly involving) unsafe might work, but
        // cloning is good enough.
        let reader = std::io::BufReader::new(port.try_clone().unwrap());

        // Implementing a test is quite easy - all you need is a big loop (which is
        // what the prototype did). Most of the complexity stems from handling:
        // - Cancellation: users may wish to stop a test, so we need some kind of
        //   messaging or semaphores to indicate cancellation.
        // - Disconnection: the user may wish to disconnect (independently of the
        //   test), or the device may disconnect. Handling this gracefully likewise
        //   adds complexity.
        // Therefore we end up with a more complex multi-thread implementation. An
        // async design is probably also feasible, tbc.

        let (tx_action, rx_action): (Sender<Action>, Receiver<Action>) = mpsc::channel();
        let (tx_command, rx_command): (Sender<Command>, Receiver<Command>) = mpsc::channel();
        // Option::None is used as a check-alive signal (see details in
        // start_receiver_thread).
        let (tx_message, rx_message): (Sender<Option<Message>>, Receiver<Option<Message>>) =
            mpsc::channel();

        let _device_thread =
            start_device_thread(rx_action, rx_message, tx_command, device_callback);
        let _sender_thread = start_sender_thread(port, rx_command);
        let _receiver_thread = start_receiver_thread(reader, tx_message);

        Ok(Device { tx_action })
    }
}

struct DevicePropertiesCollector {
    serial_number: Option<String>,
    run_time_since_last_service_hours: Option<f64>,
    last_service_month: Option<u8>,
    last_service_year: Option<u16>,
}

impl DevicePropertiesCollector {
    fn new() -> DevicePropertiesCollector {
        DevicePropertiesCollector {
            serial_number: None,
            run_time_since_last_service_hours: None,
            last_service_month: None,
            last_service_year: None,
        }
    }

    fn process(&mut self, setting: SettingMessage) -> Option<DeviceNotification> {
        match setting {
            SettingMessage::SerialNumber(serial_number) => {
                self.serial_number = Some(serial_number);
            }
            SettingMessage::RunTimeSinceService { decaminutes } => {
                self.run_time_since_last_service_hours = Some(decaminutes as f64 * 10.0 / 60.0);
            }
            SettingMessage::DateLastServiced { month, year } => {
                self.last_service_month = Some(month);
                self.last_service_year = Some(match year {
                    // For 8020As, the last known service would be around 2014
                    // (give or take non-TSI service?). But we have no idea
                    // how long 8020Ms might still be serviced (although we
                    // don't yet know if they offer similar setting extraction
                    // anyway).
                    year if year < 80 => 2000 + year as u16,
                    year => 1900 + year as u16,
                });
            }
            _ => (),
        }

        if self.serial_number.is_some()
            && self.run_time_since_last_service_hours.is_some()
            && self.last_service_month.is_some()
            && self.last_service_year.is_some()
        {
            Some(DeviceNotification::DeviceProperties {
                serial_number: self.serial_number.take().unwrap(),
                run_time_since_last_service_hours: self.run_time_since_last_service_hours.unwrap(),
                last_service_month: self.last_service_month.unwrap(),
                last_service_year: self.last_service_year.unwrap(),
            })
        } else {
            None
        }
    }
}

fn start_device_thread(
    rx_action: Receiver<Action>,
    rx_message: Receiver<Option<Message>>,
    tx_command: Sender<Command>,
    device_callback: Option<impl Fn(&DeviceNotification) + 'static + std::marker::Send>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let send_notification = |notification: &DeviceNotification| {
            if let Some(callback) = &device_callback {
                callback(notification);
            }
        };
        let send_command = |command: Command| {
            if let Err(e) = tx_command.send(command) {
                // Do not send ConnectionClosed here - if the sender has closed,
                // then we've probably lost the serial connection. In this case
                // rx_message will also close, and we use that as the canonical
                // indicator of connection loss. (rx_message is preferred for
                // this purpose as we poll it frequently, whereas tx is rare.)
                // Alternatively... the sender thread may have crashed, which
                // is obviously a disaster.
                // TODO: consider handling sender thread crashes gracefully too?
                eprintln!("tx_command failed: {e:?}");
            }
        };
        send_command(Command::EnterExternalControl);
        // TODO: loop and wait for confirmation of EnterExternalControl.

        let mut test: Option<Test> = None;
        // TODO: verify whether this is a safe assumption. It may be safer to set
        // AwaitingSpecimen and request specimen?
        let mut valve_state = ValveState::Specimen;
        let mut device_properties_collector = DevicePropertiesCollector::new();
        loop {
            // The duration is largely arbitrary, and chosen to hopefully
            // provide sufficient responsiveness.
            let message = match rx_message.recv_timeout(core::time::Duration::from_millis(50)) {
                Ok(None) => None,
                Ok(Some(msg)) => Some(msg),
                Err(error) => match error {
                    mpsc::RecvTimeoutError::Timeout => None,
                    _ => {
                        send_notification(&DeviceNotification::ConnectionClosed);
                        return;
                    }
                },
            };
            if let Some(Message::Sample(value)) = message {
                send_notification(&DeviceNotification::Sample {
                    particle_conc: value,
                });
            }

            match rx_action.try_recv() {
                Ok(action) => match action {
                    Action::StartTest {
                        config,
                        test_callback,
                    } => {
                        // Clients could send multiple StartTests (while
                        // previous tests are still running). That's OK,
                        // starting a new test is idempotent - and old tests
                        // will simply be dropped.
                        test = match Test::create_and_start(
                            config,
                            &tx_command,
                            &mut valve_state,
                            test_callback,
                        ) {
                            Ok(test) => Some(test),
                            // No need to send ConnectionClosed here - see comment in
                            // send_command above.
                            Err(_) => None,
                        };
                        send_notification(&DeviceNotification::TestStarted);
                    }
                    Action::CancelTest => {
                        send_command(Command::ClearDisplay);
                        send_notification(&DeviceNotification::TestCancelled);
                        valve_state = ValveState::AwaitingSpecimen;
                        send_command(Command::ValveSpecimen);
                        test = None;
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => (),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    send_notification(&DeviceNotification::ConnectionClosed);
                    return;
                }
            }

            let Some(message) = message else {
                continue;
            };

            if let Message::Setting(setting) = message {
                if let Some(notification) = device_properties_collector.process(setting) {
                    send_notification(&notification);
                }
                continue;
            }

            if let Some(new_state) = match message {
                Message::Response(Command::ValveAmbient) => Some(ValveState::Ambient),
                Message::Response(Command::ValveSpecimen) => Some(ValveState::Specimen),
                _ => None,
            } {
                valve_state = new_state;
            }
            test = match test {
                Some(mut test) => match test.step(message, &mut valve_state) {
                    Ok(StepOutcome::None) => Some(test),
                    Ok(StepOutcome::TestComplete) => {
                        send_notification(&DeviceNotification::TestCompleted {
                            fit_factors: test.exercise_ffs,
                        });
                        None
                    }
                    // No need to send ConnectionClosed here - see comment in
                    // send_command above.
                    Err(_) => None,
                },
                None => {
                    if let Message::Sample(value) = message {
                        send_command(Command::DisplayConcentration(value));
                    }
                    None
                }
            }
        }
    })
}

fn start_sender_thread(
    mut writer: Box<dyn serialport::SerialPort>,
    rx_command: Receiver<Command>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        let command = match rx_command.recv().unwrap().to_wire() {
            Ok(command) => command,
            Err(e) => {
                eprintln!("Not sending invalid command: {e:?}");
                continue;
            }
        };
        assert!(
            command.is_ascii(),
            "commands must be ASCII, this is a libp8020 bug (got {command})"
        );

        writer
            .write_all(command.as_bytes())
            .expect("failed to write to port");
        writer.write_all(b"\r").expect("failed to write to port");

        // Flow control is a bit laggy or broken: sending a second message within
        // approx 52ms of a previous message will result in the second message being
        // ignored (which obviously breaks subsequent assumptions).
        // To be safe I use a 100ms delay. (For my device, the threshold was right
        // around 52ms, but it may be different for other devices/computers/OS's/
        // whatever.)
        // It's also entirely possible that the problem is with my serial/USB adapter.
        // TODO: figure out if we can wait for the echo instead? This is tricky,
        // because it relies on accurate response parsing and/or good heuristics?
        std::thread::sleep(std::time::Duration::from_millis(100));
    })
}

fn start_receiver_thread(
    mut reader: std::io::BufReader<Box<dyn serialport::SerialPort>>,
    tx_message: Sender<Option<Message>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = String::new();
        loop {
            // read_line blocks until we get content OR until we reach the timeout (set
            // above). To detect that the user wishes to close a device connection, we
            // can check whether the channel is still open: if the connection is closed,
            // then device thread will close (drop) the channel refered to by tx_message.
            // The only way to check if the connection is closed is to try send()'ing.
            // Therefore we periodically send None's to the channel to check if we should
            // quit. To ensure that we check the connection sufficiently frequently, we
            // rely on a short timeout on reader.
            match reader.read_line(&mut buf) {
                Ok(0) => {
                    // This closes the channel for us, which in turns lets the
                    // device thread know that the connection is closed.
                    return;
                }
                Err(error) => match error.kind() {
                    std::io::ErrorKind::TimedOut => {
                        // "Is channel still open" check - see long comment above.
                        tx_message.send(None).unwrap();
                        continue;
                    }
                    _ => {
                        // See Ok(0) above.
                        return;
                    }
                },
                Ok(_) => (),
            };
            // BufReader removes the trailing <LR>, we need to remove the remaining <CR>.
            let message = buf.trim();
            match protocol::parse_message(message) {
                Ok(message) => tx_message.send(Some(message)).unwrap(),
                Err(e) => {
                    // TODO: log any unparseable messages to disk, to allow for later debugging.
                    println!("command parsing failed: {e:?}")
                }
            }
            buf.clear();
        }
    })
}
