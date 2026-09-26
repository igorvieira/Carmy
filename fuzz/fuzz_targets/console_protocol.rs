#![no_main]
//! Arbitrary bytes into the carmy-console/1 protocol: it must never panic, and every
//! output line must be valid JSON.
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    common::block_on(async {
        let mut output = Vec::new();
        let _ = carmy::console::serve(common::runtime(), "fuzz", data, &mut output).await;
        for line in output.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            serde_json::from_slice::<serde_json::Value>(line).expect("every output line is JSON");
        }
    });
});
