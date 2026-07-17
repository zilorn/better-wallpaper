use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use better_wallpaper_scene_format::PkgReader;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    about = "Unpack a Wallpaper Engine .pkg into a new directory and verify every output file"
)]
struct Args {
    /// Path to the .pkg archive.
    package: PathBuf,
    /// New directory that will receive the unpacked files.
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    eprintln!("Reading package {}", args.package.display());
    let data = fs::read(&args.package)
        .with_context(|| format!("failed to read package {}", args.package.display()))?;
    let package = PkgReader::parse(data).context("package validation failed")?;
    let paths = validated_output_paths(&package)?;

    // Requiring a new root prevents stale files and existing symlinks from making
    // a successful comparison misleading.
    fs::create_dir(&args.output).with_context(|| {
        format!(
            "failed to create output directory {}; it must not already exist",
            args.output.display()
        )
    })?;

    eprintln!(
        "Extracting {} entries from {} into {}",
        package.len(),
        package.version(),
        args.output.display()
    );
    for (entry, relative_path) in package.entries().iter().zip(&paths) {
        let output_path = args.output.join(relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent directory for {}", entry.filename)
            })?;
        }

        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .with_context(|| format!("failed to create output file {}", output_path.display()))?;
        output
            .write_all(package.read_entry(entry))
            .with_context(|| format!("failed to write output file {}", output_path.display()))?;
        output
            .sync_all()
            .with_context(|| format!("failed to flush output file {}", output_path.display()))?;
    }

    let mut verified_bytes = 0u64;
    for (entry, relative_path) in package.entries().iter().zip(&paths) {
        let output_path = args.output.join(relative_path);
        let unpacked = fs::read(&output_path)
            .with_context(|| format!("failed to read unpacked file {}", output_path.display()))?;
        let expected = package.read_entry(entry);
        if unpacked != expected {
            bail!(
                "verification failed for {}: expected {} bytes, read {} bytes",
                entry.filename,
                expected.len(),
                unpacked.len()
            );
        }
        verified_bytes += entry.size;
        println!("OK\t{}\t{} bytes", entry.filename, entry.size);
    }

    eprintln!(
        "Verification passed: {} files, {} bytes",
        package.len(),
        verified_bytes
    );
    Ok(())
}

/// Convert parser-normalised names to unambiguous relative filesystem paths.
/// This also rejects file/directory conflicts such as entries named `a` and
/// `a/b`, before anything is written to disk.
fn validated_output_paths(package: &PkgReader) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(package.len());
    let mut files = HashSet::with_capacity(package.len());

    for entry in package.entries() {
        let path = Path::new(&entry.filename);
        if !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            bail!("entry has an ambiguous output path: {}", entry.filename);
        }
        let path = path.to_path_buf();
        if !files.insert(path.clone()) {
            bail!(
                "entries resolve to the same output path: {}",
                entry.filename
            );
        }
        paths.push(path);
    }

    for path in &paths {
        for parent in path.ancestors().skip(1) {
            if parent.as_os_str().is_empty() {
                break;
            }
            if files.contains(parent) {
                bail!(
                    "entry path conflicts with an output directory: {}",
                    path.display()
                );
            }
        }
    }

    Ok(paths)
}
