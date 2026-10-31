#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    meinbrowser_fuzz::js(data);
});
