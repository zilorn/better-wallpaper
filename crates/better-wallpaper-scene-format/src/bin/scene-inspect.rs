use std::{fs, io::Read, path::PathBuf};

use anyhow::{Context, Result, bail};
use better_wallpaper_scene_format::{
    PkgReader, PuppetModel, SceneNodeKind, TexAtlas, TexTexture, TexVolume, analyse_scene,
    compute_compatibility, parse_scene_graph,
};
use clap::Parser;
use sha2::{Digest, Sha256};

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
    /// Inspect independent atlas images and affine frames in an unpacked TEX.
    /// Reports structural and sprite-layout checks without running the GPU.
    #[arg(long, conflicts_with_all = ["entry", "textures", "volume"])]
    atlas: bool,
    /// Inspect a bounded volume TEX without admitting it for scene playback.
    #[arg(long, conflicts_with_all = ["entry", "textures", "atlas"])]
    volume: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let data = if args.atlas || args.volume {
        let kind = if args.volume { "volume" } else { "atlas" };
        if !args
            .input
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("tex"))
        {
            bail!("--{kind} requires an unpacked .tex file");
        }
        const MAX_INPUT: u64 = 128 * 1024 * 1024;
        let mut data = Vec::new();
        fs::File::open(&args.input)?
            .take(MAX_INPUT + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 > MAX_INPUT {
            bail!("{kind} input exceeds 128 MiB");
        }
        data
    } else {
        fs::read(&args.input)
            .with_context(|| format!("failed to read input {}", args.input.display()))?
    };
    if args.volume {
        let volume = TexVolume::parse(&data).context("volume validation failed")?;
        let report = serde_json::json!({
            "path": args.input,
            "format": format!("{:?}", volume.format),
            "flags": volume.flags,
            "logical_size": [volume.width, volume.height],
            "storage_size": [volume.texture_width, volume.texture_height, volume.depth],
            "info_extra": volume.info_extra,
            "container_version": volume.container_version,
            "embedded_format": volume.free_image_format.map(|value| format!("{value:?}")),
            "mips": volume.mipmaps.iter().map(|mip| serde_json::json!({
                "width": mip.width, "height": mip.height, "depth": mip.depth,
                "decoded_bytes": mip.data.len(),
                "decoded_sha256": format!("{:x}", Sha256::digest(&mip.data)),
            })).collect::<Vec<_>>(),
            "payload_status": "validated_volume",
            "scene_playback_available": false,
            "native_visual_validation": "pending",
        });
        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "{}\t{}x{}x{}\tmips={}\tformat={:?}\tplayback=unavailable",
                args.input.display(),
                volume.texture_width,
                volume.texture_height,
                volume.depth,
                volume.mipmaps.len(),
                volume.format,
            );
        }
        return Ok(());
    }
    if args.atlas {
        let atlas = TexAtlas::parse(&data).context("atlas validation failed")?;
        let sprite_error = atlas
            .sprite_animation()
            .err()
            .map(|error| error.to_string());
        let mut report = serde_json::json!({
            "path": args.input,
            "format": format!("{:?}", atlas.format),
            "logical_size": [atlas.width, atlas.height],
            "storage_size": [atlas.texture_width, atlas.texture_height],
            "container_version": atlas.container_version,
            "frame_dimensions": atlas.frame_dimensions,
            "images": atlas.images.iter().enumerate().map(|(index, image)| serde_json::json!({
                "index": index,
                "mips": image.mipmaps.iter().map(|mip| serde_json::json!({
                    "width": mip.width, "height": mip.height, "decoded_bytes": mip.data.len(),
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>(),
            "frames": atlas.frames,
            "payload_status": if atlas.is_video { "video_unverified" } else { "validated_2d_images" },
            "native_visual_validation": "pending",
            "sprite_layout": {
                "accepted": !atlas.is_video && sprite_error.is_none(),
                "error": sprite_error,
            },
        });
        let layout_error = atlas
            .try_into_single_texture()
            .err()
            .map(|error| error.to_string());
        report["single_image_layout"] = serde_json::json!({
            "accepted": layout_error.is_none(), "error": layout_error,
        });
        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "{}\timages={}\tframes={}\tpayload={}\tsingle_image_layout={}",
                args.input.display(),
                report["images"].as_array().unwrap().len(),
                report["frames"].as_array().unwrap().len(),
                report["payload_status"],
                layout_error.as_deref().unwrap_or("accepted")
            );
        }
        return Ok(());
    }
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
