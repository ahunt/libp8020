use std::collections::HashMap;

use crate::test_config::TestConfig;

pub const OSHA: &str = include_str!("osha.csv");
pub const OSHA_LEGACY: &str = include_str!("osha_legacy.csv");
pub const OSHA_FAST_FFP: &str = include_str!("osha_fast_ffp.csv");
pub const OSHA_FAST_ELASTO: &str = include_str!("osha_fast_elasto.csv");
pub const CRASH_2_5: &str = include_str!("crash_2_5.csv");
pub const HSE_INDG_479: &str = include_str!("hse_indg_479.csv");
pub const ISO_16975_3_2017: &str = include_str!("iso_16975-3_2017.csv");
pub const LIVE_MODE_1H: &str = include_str!("live_mode_1h.csv");

const BUILTIN_CONFIGS_RAW: [&str; 8] = [
    OSHA,
    OSHA_LEGACY,
    OSHA_FAST_FFP,
    OSHA_FAST_ELASTO,
    CRASH_2_5,
    HSE_INDG_479,
    ISO_16975_3_2017,
    LIVE_MODE_1H,
];

#[derive(Debug)]
pub enum BuiltinConfigError {
    NotFound,
}

pub static BUILTIN_CONFIGS: std::sync::LazyLock<HashMap<String, crate::test_config::TestConfig>> =
    std::sync::LazyLock::new(|| {
        let mut configs = HashMap::with_capacity(BUILTIN_CONFIGS_RAW.len());
        for config_csv in BUILTIN_CONFIGS_RAW {
            let mut cursor = std::io::Cursor::new(config_csv.as_bytes());
            let config =
                TestConfig::parse_from_csv(&mut cursor).expect("builtin configs must parse");
            let id = config.id.clone();
            if configs.contains_key(&id) {
                panic!("builtin configs must each have a unique identifier");
            }
            configs.insert(id, config);
        }
        configs
    });

pub fn get_builtin_config(
    id: &String,
) -> Result<&'static crate::test_config::TestConfig, BuiltinConfigError> {
    println!("builtins: {:?}", *BUILTIN_CONFIGS);
    match (*BUILTIN_CONFIGS).get(id) {
        Some(config) => Ok(config),
        None => Err(BuiltinConfigError::NotFound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_configs_load_and_validate() {
        for config in (*BUILTIN_CONFIGS).values() {
            assert!(config.validate().is_ok());
        }
    }
}
