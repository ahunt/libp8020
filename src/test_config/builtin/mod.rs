use crate::test_config::TestConfig;

pub const OSHA: &str = include_str!("osha.csv");
pub const OSHA_LEGACY: &str = include_str!("osha_legacy.csv");
pub const OSHA_FAST_FFP: &str = include_str!("osha_fast_ffp.csv");
pub const OSHA_FAST_ELASTO: &str = include_str!("osha_fast_elasto.csv");
pub const CRASH_2_5: &str = include_str!("crash_2_5.csv");

pub const BUILTIN_CONFIGS: [&str; 5] = [
    OSHA,
    OSHA_LEGACY,
    OSHA_FAST_FFP,
    OSHA_FAST_ELASTO,
    CRASH_2_5,
];

#[derive(Debug)]
pub enum BuiltinConfigError {
    NotFound,
}

pub fn load_all_builtin_configs() -> Vec<crate::test_config::TestConfig> {
    let mut configs = Vec::with_capacity(BUILTIN_CONFIGS.len());
    for config_csv in BUILTIN_CONFIGS {
        let mut cursor = std::io::Cursor::new(config_csv.as_bytes());
        configs.push(TestConfig::parse_from_csv(&mut cursor).expect("builtin configs must parse"));
    }
    configs
}

pub fn load_builtin_config(
    short_name: &String,
) -> Result<crate::test_config::TestConfig, BuiltinConfigError> {
    for config_csv in BUILTIN_CONFIGS {
        let mut cursor = std::io::Cursor::new(config_csv.as_bytes());
        let config = TestConfig::parse_from_csv(&mut cursor).expect("builtin configs must parse");
        assert!(config.validate().is_ok(), "builtin configs must be valid");

        if config.short_name == *short_name {
            return Ok(config);
        }
    }
    Err(BuiltinConfigError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_config::TestConfig;

    #[test]
    fn test_builtin_configs_load_and_validate() {
        for config in BUILTIN_CONFIGS {
            let mut cursor = std::io::Cursor::new(config.as_bytes());
            let result = TestConfig::parse_from_csv(&mut cursor);
            assert!(result.is_ok());
            assert!(result.unwrap().validate().is_ok());
        }
    }
}
