use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use better_wallpaper_scene_format::{PkgReader, TexTexture};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Inspect a Wallpaper Engine scene package without extracting it")]
struct Args {
    /// Path to scene.pkg.
    package: PathBuf,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Print one validated UTF-8 package entry instead of the package index.
    #[arg(long, value_name = "PATH", conflicts_with = "json")]
    entry: Option<String>,
    /// Parse texture headers and report dimensions and animation metadata.
    #[arg(long, conflicts_with_all = ["json", "entry"])]
    textures: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let data = fs::read(&args.package)
        .with_context(|| format!("failed to read package {}", args.package.display()))?;
    let package = PkgReader::parse(data).context("package validation failed")?;
    if let Some(entry) = args.entry {
        let package_entry = package
            .find(&entry)
            .with_context(|| format!("package entry not found: {entry}"))?;
        let bytes = package.read_entry(package_entry);
        let text = std::str::from_utf8(bytes)
            .with_context(|| format!("package entry is not UTF-8 text: {entry}"))?;
        print!("{text}");
        return Ok(());
    }
    if args.textures {
        for entry in package.find_by_ext("tex") {
            match TexTexture::parse(package.read_entry(entry)) {
                Ok(texture) => println!(
                    "{}\t{}x{}\tstorage={}x{}\tframes={}\tduration={:.3}s\tformat={:?}",
                    entry.filename,
                    texture.width,
                    texture.height,
                    texture.texture_width,
                    texture.texture_height,
                    texture.frames.len(),
                    texture.spritesheet_duration,
                    texture.format
                ),
                Err(error) => println!("{}\tinvalid\t{}", entry.filename, error),
            }
        }
        return Ok(());
    }
    let inspection = package.inspect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&inspection)?);
    } else {
        println!(
            "Package {}: {} entries, {} bytes",
            inspection.version, inspection.file_count, inspection.total_size
        );
        for entry in inspection.entries {
            println!(
                "{}\t{}\t{} bytes\t{}",
                entry.index, entry.filename, entry.length, entry.file_type_hint
            );
        }
    }
    Ok(())
}
