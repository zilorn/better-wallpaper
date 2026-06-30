#![no_main]

use better_wallpaper_scene_format::PkgReader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|filename: &[u8]| {
    let Ok(filename_length) = u32::try_from(filename.len()) else {
        return;
    };

    let mut archive = Vec::with_capacity(28 + filename.len());
    archive.extend(8u32.to_le_bytes());
    archive.extend(b"PKGV0018");
    archive.extend(1u32.to_le_bytes());
    archive.extend(filename_length.to_le_bytes());
    archive.extend(filename);
    archive.extend(0u32.to_le_bytes());
    archive.extend(0u32.to_le_bytes());
    let _ = PkgReader::parse(archive);
});
