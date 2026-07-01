use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use better_wallpaper_scene_format::{
    ModelManifest, PkgReader, ResourceManifest, ResourceValidationReport, SceneGraph,
    analyse_scene, compute_compatibility, parse_scene_graph,
};
use clap::Parser;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(about = "Validate a Wallpaper Engine scene package and compatibility metadata")]
struct Args {
    /// Path to scene.pkg or its containing project directory.
    path: PathBuf,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
struct ValidationOutput {
    valid: bool,
    package_version: String,
    file_count: usize,
    compatibility: better_wallpaper_scene_format::CompatibilityReport,
    scene_graph: SceneGraph,
    models: ModelManifest,
    resources: ResourceManifest,
    resource_validation: ResourceValidationReport,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let package_path = if args.path.is_dir() {
        args.path.join("scene.pkg")
    } else {
        args.path
    };
    let data = fs::read(&package_path)
        .with_context(|| format!("failed to read package {}", package_path.display()))?;
    let package = PkgReader::parse(data).context("package validation failed")?;
    let Some(scene_entry) = package.find("scene.json") else {
        bail!("package validation failed: scene.json is missing");
    };
    let scene_json = package
        .read_entry_string(scene_entry)
        .context("failed to read scene.json")?;
    let metadata = analyse_scene(&scene_json).context("scene.json validation failed")?;
    let scene_graph = parse_scene_graph(&scene_json).context("scene IR conversion failed")?;
    let resources = ResourceManifest::from_package(&package);
    let models = ModelManifest::from_package(&package).context("model validation failed")?;
    let resource_validation =
        resources.validate_scene_graph_with_models(&scene_graph, Some(&models));
    let output = ValidationOutput {
        valid: resource_validation.missing.is_empty(),
        package_version: package.version().to_owned(),
        file_count: package.len(),
        compatibility: compute_compatibility(&metadata),
        scene_graph,
        models,
        resources,
        resource_validation,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "Valid scene package {} ({} entries, compatibility {:?})",
            output.package_version, output.file_count, output.compatibility.level
        );
        for feature in output.compatibility.unsupported_features {
            println!("Unsupported: {feature}");
        }
        for warning in output.compatibility.warnings {
            println!("Warning: {warning}");
        }
        for missing in output.resource_validation.missing {
            println!(
                "Missing resource: {} (referenced by {})",
                missing.path, missing.source
            );
        }
    }
    Ok(())
}
