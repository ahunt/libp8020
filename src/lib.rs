extern crate libc;
extern crate serialport;

use std::io::BufRead;
use std::str::FromStr;

pub struct Device {
    port: Box<dyn serialport::SerialPort>,
    reader: std::io::BufReader<Box<dyn serialport::SerialPort>>,
}

#[repr(C)]
pub enum TestState {
    Pending,
    StartedExercise(usize),
    Finished,
}

#[repr(C)]
pub enum TestNotification {
    /// StateChange indicates that the test has changed state, e.g. a new
    /// exercise was started. Note that just because a given exercise (or
    /// the entire test) was completed, it is not safe to assume that all
    /// data for that exercise (or the entire test) is available yet.
    StateChange(TestState),
    /// ExerciseResult indicates that the FF for exercise N was M.
    ExerciseResult(usize, f64),
    /// RawSample indicates a fresh reading from the PC. It is safe to assume
    /// that it was delivered 1s (plus/minus the 8020's internal delays) after
    /// the previous RawReading. This is simply the latest sample, no more,
    /// no less - i.e. it might be part of the ambient or specimen purge,
    /// or from the actually sampling period.
    // TODO: check specs for what the actual allowed range is.
    // TODO: move this into a new Device-specific callback. Raw samples are
    // available as soon as we've connected, and I'd like to see raw samples
    // prior to starting a test, as it allows you to detect if particle levels
    // in the mask haven't settled yet (I'm not convinced that this is a real
    // issue, but being able to visualise this data will help verify).
    RawSample(f64),
}

#[repr(C)]
pub struct TestConfig {
    // 255 exercises ought to be enough for anyone, but I see no good reason not
    // to use the platform's native size.
    exercise_count: usize,
    // TODO: introduce a way to represent variable exercise setups (along with the
    // fast protocols).
    ambient_purge_time: usize,
    ambient_sample_time: usize,
    specimen_purge_time: usize,
    specimen_sample_time: usize,
    test_callback: Option<extern "C" fn(&TestNotification, *mut std::ffi::c_void) -> ()>,
    test_callback_data: *mut std::ffi::c_void,
}

impl TestConfig {
    /// Returns a new TestConfig with default sample/purge timings. Callers should
    /// populate any and all callbacks that they require prior to passing the config
    /// to run_test.
    #[export_name = "test_config_new"]
    pub extern "C" fn new(exercise_count: usize) -> *mut TestConfig {
        Box::leak(Box::new(TestConfig {
            exercise_count: exercise_count,
            ambient_purge_time: 4,
            ambient_sample_time: 5,
            specimen_purge_time: 11,
            specimen_sample_time: 40,
            test_callback: Option::None,
            test_callback_data: 0 as *mut std::ffi::c_void,
        }))
    }

    // TODO: add test_config_free
}

#[repr(C)]
pub struct TestResult {
    exercise_count: usize,
    fit_factors: *mut f64,
}

// TODO: add impl TestResult with p8020_test_result_free() for FFI clients.

// TODO: refactor this into something sensible.
// TODO: introduce proper error handling.
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

#[derive(Clone)]
struct Exercise {
    ambient_purges_done: usize,
    ambient_samples: std::vec::Vec<f64>,
    specimen_switch_received: bool,
    specimen_purges_done: usize,
    specimen_samples: std::vec::Vec<f64>,
}

impl Exercise {
    fn new(test_config: &TestConfig) -> Exercise {
        Exercise {
            ambient_purges_done: 0,
            ambient_samples: Vec::with_capacity(test_config.ambient_sample_time),
            specimen_switch_received: false,
            specimen_purges_done: 0,
            specimen_samples: Vec::with_capacity(test_config.specimen_sample_time),
        }
    }
}

impl Device {
    /// Connects to the 8020A at the specified path, and returns a new Device
    /// representing this connection.
    /// Non-rust callers must call device_free to release the returned device.
    // TODO: add proper error handling (once I've figured out what an
    // appropriate approach is in conjunction with FFI)
    // TODO: switch to a builder pattern for params such as baud rate.
    // Hopefully no one is using other baud rates, but it'd be interesting to
    // experiment regardless.
    #[export_name = "device_connect"]
    pub extern "C" fn connect(path_raw: *const libc::c_char) -> *mut Device {
        let path_cstr = unsafe { std::ffi::CStr::from_ptr(path_raw) };
        // Get copy-on-write Cow<'_, str>, then guarantee a freshly-owned String allocation
        let path = String::from_utf8_lossy(path_cstr.to_bytes()).to_string();

        // See "PortaCount Plus Model 8020 Technical Addendum" for specs.
        // Note: baud is configurable on the devices itself, 1200 is the default.
        let port = serialport::new(path, /* baud_rate */ 1200)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::Hardware)
            .timeout(core::time::Duration::new(15, 0))
            .open()
            .expect("Unable to open serial port, sorry");

        let reader = std::io::BufReader::new(port.try_clone().unwrap());
        Box::leak(Box::new(Device {
            port: port,
            reader: reader,
        }))
    }

    /// Run a fit test. This function - and all its callbacks - is/are entirely
    /// synchronous (see also comment on TestConfig).
    // TODO: split this into an FFI vs non-FFI version, where only the FFI version has to leak.
    #[export_name = "device_run_test"]
    pub extern "C" fn run_test(self: &mut Self, test_config: &TestConfig) -> *mut TestResult {
        // TODO: rewrite all this. It works, but it's totally inelegant.

        // TODO: do some probing first to determine whether the Portacount is
        // already in external control mode etc. Ideally we would probe the device
        // during initial connection.
        send(&mut self.port, "J"); // Invoke External Control

        // Flow control is a bit laggy or broken: sending a second message within
        // approx 52ms of a previous message will result in the second message being
        // ignored (which obviously breaks subsequent assumptions).
        // To be safe I use a 100ms delay. (For my device, the threshold was right
        // around 52ms, but it may be different for other devices/computers/OS's/
        // whatever.)
        // It's also entirely possible that the problem is with my serial/USB adapter.
        std::thread::sleep(std::time::Duration::from_millis(100));

        send(&mut self.port, "VN"); // Switch valve on
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Additional exercise is used for the final ambient samples (specimen samples are left empty).
        let exercises = &mut vec![Exercise::new(&test_config); test_config.exercise_count + 1]
            .into_boxed_slice();
        let mut current_exercise = 0;

        // Get rid of any buffered junk - this is possible if the device was already
        // in external control mode. And skip straight to where we switched to
        // ambient sampling.
        for line in (&mut self.reader).lines() {
            if line.unwrap().trim() == "VN" {
                break;
            }
        }

        send(&mut self.port, "N01"); // Exercise 1
        std::thread::sleep(std::time::Duration::from_millis(100));
        send(&mut self.port, "I01000000"); // In progress. Other indicators are also possible, but uninteresting for my use case.
        std::thread::sleep(std::time::Duration::from_millis(100));
        send(&mut self.port, "B40"); // Beep
        std::thread::sleep(std::time::Duration::from_millis(100));

        for line in (&mut self.reader).lines() {
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
                    if current_exercise == test_config.exercise_count + 1 {
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
                    if current_exercise != test_config.exercise_count {
                        if let Some(callback) = &test_config.test_callback {
                            let notification = TestNotification::StateChange(
                                TestState::StartedExercise(current_exercise),
                            );
                            callback(&notification, test_config.test_callback_data);
                        }
                    }

                    continue;
                }
                ref m if m.starts_with("B") => {
                    // Ignore - the Portacount mirrors these.
                    continue;
                }
                ref m if m.starts_with("N") => {
                    // Ignore - the Portacount mirrors these.
                    continue;
                }
                ref m if m.starts_with("I") => {
                    // Ignore - the Portacount mirrors these.
                    continue;
                }
                _ => (),
            }

            let value = match f64::from_str(message) {
                Ok(res) => {
                    if let Some(callback) = &test_config.test_callback {
                        let notification = TestNotification::RawSample(res);
                        callback(&notification, test_config.test_callback_data);
                    }

                    res
                }
                Err(_) => {
                    eprintln!("Unexpected message received: {}", message);
                    continue;
                }
            };

            if current.ambient_purges_done < test_config.ambient_purge_time {
                current.ambient_purges_done += 1;
            } else if current.ambient_samples.len() < test_config.ambient_sample_time {
                current.ambient_samples.push(value);
                if current.ambient_samples.len() == test_config.ambient_sample_time {
                    send(&mut self.port, "VF"); // Switch valve off

                    // FF calculation
                    // TODO: extract this.
                    if current_exercise > 0 {
                        let ambient_avg = (exercises[current_exercise - 1]
                            .ambient_samples
                            .iter()
                            .sum::<f64>()
                            + exercises[current_exercise]
                                .ambient_samples
                                .iter()
                                .sum::<f64>())
                            / ((exercises[current_exercise - 1].ambient_samples.len()
                                + exercises[current_exercise].ambient_samples.len())
                                as f64);
                        let specimen_avg = exercises[current_exercise - 1]
                            .specimen_samples
                            .iter()
                            .sum::<f64>()
                            / (exercises[current_exercise - 1].specimen_samples.len() as f64);
                        let fit_factor = ambient_avg / specimen_avg;
                        println!("Exercise {}: FF {:.1}", current_exercise, fit_factor);

                        if let Some(callback) = &test_config.test_callback {
                            let notification =
                                TestNotification::ExerciseResult(current_exercise - 1, fit_factor);
                            callback(&notification, test_config.test_callback_data);
                        }
                    }
                    if current_exercise == test_config.exercise_count {
                        break;
                    }
                }
            } else if !current.specimen_switch_received {
                eprintln!("Received (unexpected) ambient sample after requesting valve switch. That's fine, it just means something was slow.");
            } else if current.specimen_purges_done < test_config.specimen_purge_time {
                current.specimen_purges_done += 1;
            } else if current.specimen_samples.len() < test_config.specimen_sample_time {
                current.specimen_samples.push(value);
                if current.specimen_samples.len() == test_config.specimen_sample_time {
                    send(&mut self.port, "VN"); // Switch valve on
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    // current_exercise is incremented later, and we also need to convert from zero to human indexing.
                    send(
                        &mut self.port,
                        format!("N{:02}", current_exercise + 2).as_str(),
                    ); // Exercise N
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    send(&mut self.port, "B05"); // Beep
                }
            } else {
                eprintln!("Received (unexpected) specimen sample after requesting valve switch. That's fine, it just means something was slow.");
            }
        }

        send(&mut self.port, "G"); // Release from external control

        let mut fit_factors = vec![0.0f64; test_config.exercise_count];
        for i in 0..test_config.exercise_count {
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
            fit_factors[i] = fit_factor;
        }
        // TODO: print avg FF.

        let ret = Box::leak(Box::new(TestResult {
            exercise_count: test_config.exercise_count,
            fit_factors: fit_factors.as_mut_ptr(),
        }));
        std::mem::forget(fit_factors);
        ret
    }

    #[export_name = "device_free"]
    pub extern "C" fn free(self: &mut Self) {
        unsafe {
            drop(Box::from_raw(self));
        }
    }
}
