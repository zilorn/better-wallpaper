#![no_main]

use better_wallpaper_scene_format::PkgReader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = PkgReader::parse(data);
});
