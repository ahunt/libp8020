#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = p8020::test_config::TestConfig::parse_from_csv(&mut std::io::Cursor::new(&data));
});
