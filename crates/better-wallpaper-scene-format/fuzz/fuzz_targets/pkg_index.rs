#![no_main]

use better_wallpaper_scene_format::PkgReader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut archive = Vec::with_capacity(16 + data.len());
    archive.extend(8u32.to_le_bytes());
    archive.extend(b"PKGV0018");
    archive.extend(data);

    if let Ok(pkg) = PkgReader::parse(archive) {
        for entry in pkg.entries() {
            let _ = pkg.read_entry(entry);
        }
    }
});
