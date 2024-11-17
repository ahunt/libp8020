use crate::test_config::builtin::*;
use crate::test_config::TestConfig;
use std::ffi::CString;
use std::os::raw::c_char;

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
