extern crate libc;
extern crate serialport;

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
    ExerciseResult(usize, f32),
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
    RawSample(u64),
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
    #[export_name = "device_run_test"]
    pub extern "C" fn run_test(self: &Self, _config: &TestConfig) -> bool {
        // TODO: implement this.
        return false;
    }

    #[export_name = "device_free"]
    pub extern "C" fn free(self: &mut Self) {
      unsafe {drop(Box::from_raw(self));}
    }
}
