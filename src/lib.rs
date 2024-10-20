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
    /// TODO: check specs for what the actual allowed range is.
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

//#[no_mangle]

impl TestConfig {
    /// Returns a new TestConfig with default sample/purge timings. Callers should
    /// populate any and all callbacks that they require prior to passing the config
    /// to run_test.
    #[export_name = "test_config_new"]
    pub extern "C" fn new(exercise_count: usize) -> TestConfig {
        TestConfig {
            exercise_count: exercise_count,
            ambient_purge_time: 4,
            ambient_sample_time: 5,
            specimen_purge_time: 11,
            specimen_sample_time: 40,
            test_callback: Option::None,
            test_callback_data: 0 as *mut std::ffi::c_void,
        }
    }
}

/// Run a fit test. This function - and all its callbacks - is/are entirely
/// synchronous (see also comment on TestConfig).
#[no_mangle]
pub extern "C" fn run_test(_config: &TestConfig) -> bool {
    // TODO: implement this.
    return false;
}
