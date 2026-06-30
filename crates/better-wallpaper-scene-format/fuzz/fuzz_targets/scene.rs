#![no_main]

use better_wallpaper_scene_format::{analyse_scene, compute_compatibility};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data)
        && let Ok(metadata) = analyse_scene(input)
    {
        let _ = compute_compatibility(&metadata);
    }
});
