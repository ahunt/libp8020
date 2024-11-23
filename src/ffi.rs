extern crate libc;

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};

use crate::{Action,Device,DeviceNotification};
use crate::test::TestNotification;
use crate::test_config::TestConfig;
use crate::test_config::builtin::BUILTIN_CONFIGS;

/// FFI wrapper for Device.
pub struct P8020Device {
    device: Device,
    rx_done: Receiver<()>,
}

// A (C) void* wrapper, which can be (un)safely transmitted across threads.
struct FFICallbackDataHandle(*mut std::ffi::c_void);
unsafe impl Send for FFICallbackDataHandle {}
unsafe impl Sync for FFICallbackDataHandle {}

impl FFICallbackDataHandle {
    fn get(self: &Self) -> *mut std::ffi::c_void {
        self.0
    }
}

#[repr(C)]
pub struct TestResult {
    exercise_count: usize,
    fit_factors: *mut f64,
}
// TODO: add impl TestResult with p8020_test_result_free() for FFI clients.

impl P8020Device {
    /// Connects to the 8020A at the specified path, and returns a new Device
    /// representing this connection.
    /// Non-rust callers must call device_free to release the returned device.
    #[export_name = "p8020_device_connect"]
    pub extern "C" fn connect(
        path_raw: *const libc::c_char,
        callback: extern "C" fn(&DeviceNotification, *mut std::ffi::c_void) -> (),
        callback_data: *mut std::ffi::c_void,
    ) -> *mut P8020Device {
        let path_cstr = unsafe { std::ffi::CStr::from_ptr(path_raw) };
        let path = String::from_utf8_lossy(path_cstr.to_bytes()).to_string();

        let callback_data = FFICallbackDataHandle(callback_data);
        let (tx_done, rx_done): (Sender<()>, Receiver<()>) = mpsc::channel();
        let device_callback = move |notification: &DeviceNotification| {
            callback(&notification, callback_data.get());
            if let DeviceNotification::TestCompleted = notification {
                tx_done.send(()).unwrap();
            }
        };
        match Device::connect_path(path, Some(device_callback)) {
            Ok(device) => Box::into_raw(Box::new(P8020Device {
                device: device,
                rx_done: rx_done,
            })),
            Err(_) => std::ptr::null_mut(),
        }
    }

    /// Run a fit test (this API will change a lot soon).
    #[export_name = "p8020_device_run_test"]
    pub extern "C" fn run_test(
        self: &mut Self,
        test_config: &TestConfig,
        callback: extern "C" fn(&TestNotification, *mut std::ffi::c_void) -> (),
        callback_data: *mut std::ffi::c_void,
    ) -> *mut TestResult {
        let callback_data = FFICallbackDataHandle(callback_data);
        let test_callback = move |notification: &TestNotification| {
            callback(&notification, callback_data.get());
        };
        self.device
            .tx_action
            .send(Action::StartTest {
                config: test_config.clone(),
                test_callback: Some(Box::new(test_callback)),
            })
            .expect("device connection is (probably) gone");
        self.rx_done.recv().expect("rx_done failed");

        // TODO: populate this.
        let mut fit_factors = vec![0.0f64; 1];
        let ret = Box::leak(Box::new(TestResult {
            exercise_count: 1,
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

#[export_name = "p8020_test_config_builtin_count"]
pub extern "C" fn builtin_count() -> usize {
    BUILTIN_CONFIGS.len()
}

#[export_name = "p8020_test_config_builtin_load"]
pub extern "C" fn load_builtin_config(short_name_raw: *const libc::c_char) -> *mut TestConfig {
    let short_name_cstr = unsafe { std::ffi::CStr::from_ptr(short_name_raw) };
    let short_name = String::from_utf8_lossy(short_name_cstr.to_bytes()).to_string();

    for config_csv in BUILTIN_CONFIGS {
        let mut cursor = std::io::Cursor::new(config_csv.as_bytes());
        let config = TestConfig::parse_from_csv(&mut cursor).expect("builtin configs must parse");
        assert!(config.validate().is_ok(), "builtin configs must be valid");

        if config.short_name == short_name {
            return Box::into_raw(Box::new(config));
        }
    }
    std::ptr::null_mut()
}

#[export_name = "p8020_test_config_exercise_count"]
pub extern "C" fn config_exercise_count(config: &TestConfig) -> usize {
    config.exercise_count()
}

/// Returns the name of the specified exercise. Returned pointers must be freed
/// using p8020_test_config_exercise_name_free().
#[export_name = "p8020_test_config_exercise_name"]
pub extern "C" fn config_exercise_name(config: &TestConfig, index: usize) -> *mut c_char {
    let name = config.exercise_names().remove(index);
    CString::new(name)
        .expect("builtin test config names should not contain NULLs")
        .into_raw()
}

#[export_name = "p8020_test_config_exercise_name_free"]
pub extern "C" fn config_exercise_name_free(name: &mut c_char) {
    unsafe {
        drop(Box::from_raw(name));
    }
}

#[export_name = "p8020_test_config_free"]
pub extern "C" fn config_free(config: &mut TestConfig) {
    unsafe {
        drop(Box::from_raw(config));
    }
}
