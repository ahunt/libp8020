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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_config::TestConfig;

    #[test]
    fn test_BUILTIN_CONFIGS_load_and_validate() {
        for config in BUILTIN_CONFIGS {
            let mut cursor = std::io::Cursor::new(config.as_bytes());
            let result = TestConfig::parse_from_csv(&mut cursor);
            assert!(result.is_ok());
            assert!(result.unwrap().validate().is_ok());
        }
    }
}
