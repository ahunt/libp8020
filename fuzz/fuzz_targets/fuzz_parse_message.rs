#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(str) = std::str::from_utf8(data) else {
        return;
    };
    let _ = p8020::protocol::parse_message(str);
});
