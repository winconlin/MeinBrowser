#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| meinbrowser_fuzz::webidl(data));
