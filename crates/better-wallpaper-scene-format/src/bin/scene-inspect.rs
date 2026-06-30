use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use better_wallpaper_scene_format::PkgReader;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Inspect a Wallpaper Engine scene package without extracting it")]
struct Args {
    /// Path to scene.pkg.
    package: PathBuf,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let data = fs::read(&args.package)
        .with_context(|| format!("failed to read package {}", args.package.display()))?;
    let package = PkgReader::parse(data).context("package validation failed")?;
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
