#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(config) =
        p8020::test_config::TestConfig::parse_from_csv(&mut std::io::Cursor::new(&data))
    {
        let _ = config.validate();
    }
});
