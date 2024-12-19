extern crate libc;

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use crate::test::TestNotification;
use crate::test_config::builtin::BUILTIN_CONFIGS;
use crate::test_config::TestConfig;
use crate::{Action, Device, DeviceNotification, DeviceProperties};

#[repr(C)]
pub enum P8020DeviceNotification {
    Sample {
        #[allow(dead_code)] // Used via FFI
        particle_conc: f64,
    },
    ConnectionClosed,
    // Indicates that device properties can now be retrieved via
    // p8020_device_get_properties.
    DevicePropertiesAvailable,
}

/// FFI wrapper for Device.
pub struct P8020Device {
    device: Device,
    // Receiver for test completion signal. OK(fit_factors) on successful
    // completion, Err(()) on cancellation.
    rx_done: Receiver<Result<Vec<f64>, ()>>,
    device_properties: Arc<Mutex<Option<DeviceProperties>>>,
}

#[allow(dead_code)] // All fields read via FFI
pub struct P8020DeviceProperties {
    pub serial_number: *const libc::c_char,
    pub run_time_since_last_service_hours: f64,
    pub last_service_month: u8,
    pub last_service_year: u16,
}

impl P8020DeviceProperties {
    #[export_name = "p8020_device_properties_free"]
    pub extern "C" fn free(&mut self) {
        unsafe {
            drop(Box::from_raw(self));
        }
    }
}

// A (C) void* wrapper, which can be (un)safely transmitted across threads.
struct FFICallbackDataHandle(*mut std::ffi::c_void);
unsafe impl Send for FFICallbackDataHandle {}
unsafe impl Sync for FFICallbackDataHandle {}

impl FFICallbackDataHandle {
    fn get(&self) -> *mut std::ffi::c_void {
        self.0
    }
}

#[repr(C)]
pub struct P8020TestResult {
    exercise_count: usize,
    fit_factors: *mut f64,
    fit_factors_length: usize,
    fit_factors_capacity: usize,
}

impl P8020Device {
    /// Connects to the 8020A at the specified path, and returns a new Device
    /// representing this connection.
    /// Non-rust callers must call device_free to release the returned device.
    #[export_name = "p8020_device_connect"]
    pub extern "C" fn connect(
        path_raw: *const libc::c_char,
        callback: extern "C" fn(&P8020DeviceNotification, *mut std::ffi::c_void) -> (),
        callback_data: *mut std::ffi::c_void,
    ) -> *mut P8020Device {
        let path_cstr = unsafe { std::ffi::CStr::from_ptr(path_raw) };
        let path = String::from_utf8_lossy(path_cstr.to_bytes()).to_string();

        let callback_data = FFICallbackDataHandle(callback_data);
        let (tx_done, rx_done) = mpsc::channel();
        // Use an Arc<Mutex> to share device_properties from our closure to
        // P8020Device. This is extremely inelegant, and I wonder if there's a
        // rustier way to do this.
        let device_properties = Arc::new(Mutex::new(None));
        let device_properties_write = device_properties.clone();
        let device_callback = move |notification: &DeviceNotification| {
            if let Some(notification) = match notification {
                DeviceNotification::Sample { particle_conc } => {
                    Some(P8020DeviceNotification::Sample {
                        particle_conc: *particle_conc,
                    })
                }
                DeviceNotification::ConnectionClosed => {
                    Some(P8020DeviceNotification::ConnectionClosed)
                }
                DeviceNotification::DeviceProperties(updated_properties) => {
                    *device_properties_write.lock().unwrap() = Some(updated_properties.clone());
                    Some(P8020DeviceNotification::DevicePropertiesAvailable)
                }
                DeviceNotification::TestStarted
                | DeviceNotification::TestCancelled
                | DeviceNotification::TestCompleted { .. } => None,
            } {
                callback(&notification, callback_data.get());
            }
            if let DeviceNotification::TestCompleted { fit_factors } = notification {
                tx_done.send(Ok(fit_factors.clone())).unwrap();
            } else if let DeviceNotification::TestCancelled = notification {
                tx_done.send(Err(())).unwrap();
            }
        };
        match Device::connect_path(path, Some(device_callback)) {
            Ok(device) => Box::into_raw(Box::new(P8020Device {
                device,
                rx_done,
                device_properties,
            })),
            Err(_) => std::ptr::null_mut(),
        }
    }

    /// Run a fit test (this API will change a lot soon).
    #[export_name = "p8020_device_run_test"]
    pub extern "C" fn run_test(
        &mut self,
        test_config: &TestConfig,
        callback: extern "C" fn(&TestNotification, *mut std::ffi::c_void) -> (),
        callback_data: *mut std::ffi::c_void,
    ) -> *mut P8020TestResult {
        let callback_data = FFICallbackDataHandle(callback_data);
        let test_callback = move |notification: &TestNotification| {
            callback(notification, callback_data.get());
        };
        self.device
            .tx_action
            .send(Action::StartTest {
                config: test_config.clone(),
                test_callback: Some(Box::new(test_callback)),
            })
            .expect("device connection is (probably) gone");

        let Ok(mut fit_factors) = self.rx_done.recv().expect("rx_done failed") else {
            return std::ptr::null_mut();
        };

        // Could be switched to Vec.into_raw_parts() once it become stable:
        // https://github.com/rust-lang/rust/issues/65816
        let (data, length, capacity) = (
            fit_factors.as_mut_ptr(),
            fit_factors.len(),
            fit_factors.capacity(),
        );
        std::mem::forget(fit_factors);
        Box::leak(Box::new(P8020TestResult {
            exercise_count: 1,
            fit_factors: data,
            fit_factors_length: length,
            fit_factors_capacity: capacity,
        }))
    }

    /// Returns cached deviced properties, or NULL if not available yet. No data
    /// will be available until P8020DeviceNotification::DevicePropertiesAvailable
    /// has been sent.
    #[export_name = "p8020_device_get_properties"]
    pub extern "C" fn get_properties(&self) -> *mut P8020DeviceProperties {
        let Some(ref device_properties) = *self.device_properties.lock().unwrap() else {
            return std::ptr::null_mut();
        };
        let serial_number = CString::new(device_properties.serial_number.clone())
            .expect("serial number should never contain NULLs")
            .into_raw();
        Box::leak(Box::new(P8020DeviceProperties {
            serial_number,
            run_time_since_last_service_hours: device_properties.run_time_since_last_service_hours,
            last_service_month: device_properties.last_service_month,
            last_service_year: device_properties.last_service_year,
        }))
    }

    #[export_name = "p8020_device_free"]
    pub extern "C" fn free(&mut self) {
        unsafe {
            drop(Box::from_raw(self));
        }
    }
}

impl P8020TestResult {
    #[export_name = "p8020_test_result_free"]
    pub extern "C" fn test_result_free(&mut self) {
        unsafe {
            let _ = Vec::from_raw_parts(
                self.fit_factors,
                self.fit_factors_length,
                self.fit_factors_capacity,
            );
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
