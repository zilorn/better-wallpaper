//! Read local TEX assets without starting Wallpaper Engine or a GPU context.
use better_wallpaper_scene_format::texture::TexTexture;
use std::{env, fs, io, path::Path};

#[derive(Default)]
struct Counts {
    total: usize,
    validated: usize,
    parse_errors: usize,
    payload_errors: usize,
}

fn walk(path: &Path, counts: &mut Counts) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        let mut entries = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        entries.sort();
        for path in entries {
            walk(&path, counts)?;
        }
    } else if metadata.is_file() && path.extension().is_some_and(|e| e == "tex") {
        counts.total += 1;
        let bytes = fs::read(path)?;
        match TexTexture::parse(&bytes) {
            Ok(texture) => {
                let payload = texture.to_texture_image().map(|image| image.levels.len());
                if payload.is_ok() {
                    counts.validated += 1;
                } else {
                    counts.payload_errors += 1;
                }
                println!(
                    "{}\tversion={} mips={} frames={} first_frame={:?} payload={payload:?}",
                    path.display(),
                    texture.container_version,
                    texture.mipmaps.len(),
                    texture.frames.len(),
                    texture.frames.first(),
                );
            }
            Err(error) => {
                counts.parse_errors += 1;
                println!("{}\tparse_error={error}", path.display());
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::args_os()
        .nth(1)
        .ok_or("usage: texture-audit <local TEX file or directory>")?;
    let mut counts = Counts::default();
    walk(Path::new(&root), &mut counts)?;
    if counts.total == 0 {
        return Err("No TEX files found (symlinks are not followed)".into());
    }
    println!(
        "total={} validated_2d_payloads={} parse_errors={} payload_errors={}",
        counts.total, counts.validated, counts.parse_errors, counts.payload_errors,
    );
    // Errors are inventory results; successful payload validation is not playback.
    Ok(())
}
