use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use better_wallpaper_scene_format::{
    PkgReader, PuppetModel, SceneNodeKind, TexTexture, analyse_scene, compute_compatibility,
    parse_scene_graph,
};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Inspect a Wallpaper Engine scene package or unpacked scene/TEX file")]
struct Args {
    /// Path to scene.pkg, an unpacked scene.json, or an unpacked .tex file.
    input: PathBuf,
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
    let data = fs::read(&args.input)
        .with_context(|| format!("failed to read input {}", args.input.display()))?;
    if args
        .input
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("scene.json"))
    {
        if args.entry.is_some() || args.textures {
            bail!("--entry and --textures cannot be used with an unpacked scene.json");
        }
        let scene_json = std::str::from_utf8(&data).context("scene.json is not valid UTF-8")?;
        let metadata = analyse_scene(scene_json).context("scene metadata validation failed")?;
        let graph = parse_scene_graph(scene_json).context("scene IR conversion failed")?;
        let compatibility = compute_compatibility(&metadata);
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": args.input,
                    "metadata": metadata,
                    "compatibility": compatibility,
                    "scene_graph": graph,
                }))?
            );
        } else {
            let image_nodes = graph
                .nodes
                .iter()
                .filter(|node| matches!(node.kind, SceneNodeKind::Image(_)))
                .count();
            println!(
                "{}\tobjects={}\timages={}\tunsupported={}\tcompatibility={:?}",
                args.input.display(),
                graph.nodes.len(),
                image_nodes,
                graph.unsupported_features.len(),
                compatibility.level,
            );
        }
        return Ok(());
    }
    if args
        .input
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tex"))
    {
        if args.entry.is_some() {
            bail!("--entry cannot be used with an unpacked texture");
        }
        let texture = TexTexture::parse(&data).context("texture validation failed")?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": args.input,
                    "width": texture.width,
                    "height": texture.height,
                    "storage_width": texture.texture_width,
                    "storage_height": texture.texture_height,
                    "format": format!("{:?}", texture.format),
                    "embedded_format": texture.free_image_format.map(|value| format!("{value:?}")),
                    "mip_levels": texture.mipmaps.iter().map(|mip| serde_json::json!({
                        "width": mip.width,
                        "height": mip.height,
                        "decoded_bytes": mip.data.len(),
                    })).collect::<Vec<_>>(),
                    "frames": texture.frames.len(),
                    "duration_seconds": texture.spritesheet_duration,
                    "is_video": texture.is_video,
                }))?
            );
        } else {
            println!(
                "{}\t{}x{}\tstorage={}x{}\tmips={}\tframes={}\tduration={:.3}s\tformat={:?}\tembedded={:?}\tvideo={}",
                args.input.display(),
                texture.width,
                texture.height,
                texture.texture_width,
                texture.texture_height,
                texture.mipmaps.len(),
                texture.frames.len(),
                texture.spritesheet_duration,
                texture.format,
                texture.free_image_format,
                texture.is_video,
            );
        }
        return Ok(());
    }
    if args
        .input
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mdl"))
    {
        if args.entry.is_some() || args.textures {
            bail!("--entry and --textures cannot be used with an unpacked puppet model");
        }
        let model = PuppetModel::parse(&data).context("puppet model validation failed")?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": args.input,
                    "vertices": model.vertices.len(),
                    "indices": model.indices.len(),
                    "bones": model.bones.len(),
                    "animations": model.animations.iter().map(|animation| serde_json::json!({
                        "id": animation.id,
                        "name": animation.name,
                        "fps": animation.fps,
                        "frames": animation.frame_count,
                    })).collect::<Vec<_>>(),
                }))?
            );
        } else {
            println!(
                "{}\tvertices={}\tindices={}\tbones={}\tanimations={}",
                args.input.display(),
                model.vertices.len(),
                model.indices.len(),
                model.bones.len(),
                model.animations.len(),
            );
        }
        return Ok(());
    }
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
