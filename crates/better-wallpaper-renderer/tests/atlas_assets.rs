use better_wallpaper_renderer::{Scene2dOptions, build_scene_2d_plan, resolve_scene_2d_assets};
use better_wallpaper_scene_format::{
    CompatibilityLevel, PkgReader, analyse_scene_with_package, compute_compatibility,
    parse_scene_graph,
};
use std::{path::Path, sync::Arc};

fn fixture(name: &str) -> PkgReader {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/scene-behavior")
        .join(name)
        .join("scene.pkg");
    PkgReader::parse(std::fs::read(path).unwrap()).unwrap()
}

fn scene(pkg: &PkgReader) -> String {
    pkg.read_entry_string(pkg.find("scene.json").unwrap())
        .unwrap()
        .into_owned()
}

fn assets(pkg: &PkgReader, scene: &str) -> better_wallpaper_renderer::Scene2dAssets {
    let graph = parse_scene_graph(scene).unwrap();
    let plan = build_scene_2d_plan(
        &graph,
        Scene2dOptions {
            viewport_width: 32,
            viewport_height: 32,
        },
    )
    .unwrap();
    resolve_scene_2d_assets(pkg, plan).unwrap()
}

#[test]
fn atlas_resources_and_compatibility_use_the_same_admission_rules() {
    for name in ["multi-image", "rotated", "skewed", "flipped-x", "flipped-y"] {
        let pkg = fixture(name);
        let scene = scene(&pkg);
        let assets = assets(&pkg, &scene);
        assert_eq!(assets.skipped_nodes, 0, "{name}");
        assert_eq!(assets.draws.len(), 1);
        let draw = &assets.draws[0];
        assert_eq!(
            draw.additional_textures.len(),
            usize::from(name == "multi-image")
        );
        assert_eq!(draw.animation.as_ref().unwrap().frames.len(), 2);
        let meta = analyse_scene_with_package(&scene, &pkg).unwrap();
        assert!(meta.has_sprite_animation);
        assert!(meta.unsupported_textures.is_empty());
        assert_eq!(compute_compatibility(&meta).level, CompatibilityLevel::L2);
    }
}

#[test]
fn repeated_draws_share_all_decoded_atlas_images() {
    let pkg = fixture("multi-image");
    let scene = r#"{"general":{"orthogonalprojection":{"width":32,"height":32}},"objects":[
        {"image":"models/atlas.json","origin":"16 16 0","size":"32 32"},
        {"image":"models/atlas.json","origin":"16 16 0","size":"32 32"}] }"#;
    let assets = assets(&pkg, scene);
    assert_eq!(assets.draws.len(), 2);
    assert!(Arc::ptr_eq(
        &assets.draws[0].texture.levels[0].data,
        &assets.draws[1].texture.levels[0].data
    ));
    assert!(Arc::ptr_eq(
        &assets.draws[0].additional_textures[0].levels[0].data,
        &assets.draws[1].additional_textures[0].levels[0].data
    ));
}

#[test]
fn invalid_later_frame_reports_the_limit_and_skips_the_whole_draw() {
    let original = fixture("rotated");
    let mut entries = original
        .entries()
        .iter()
        .map(|entry| (entry.filename.clone(), original.read_entry(entry).to_vec()))
        .collect::<Vec<_>>();
    let tex = &mut entries
        .iter_mut()
        .find(|(name, _)| name.ends_with(".tex"))
        .unwrap()
        .1;
    let block = tex.windows(9).position(|b| b == b"TEXS0002\0").unwrap();
    tex[block + 49..block + 53].copy_from_slice(&0.0_f32.to_le_bytes());
    let mut bytes = Vec::new();
    fn string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend((value.len() as u32).to_le_bytes());
        bytes.extend(value.as_bytes());
    }
    string(&mut bytes, "PKGV0001");
    bytes.extend((entries.len() as u32).to_le_bytes());
    let mut offset = 0_u32;
    for (name, data) in &entries {
        string(&mut bytes, name);
        bytes.extend(offset.to_le_bytes());
        bytes.extend((data.len() as u32).to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in entries {
        bytes.extend(data);
    }
    let pkg = PkgReader::parse(bytes).unwrap();
    let scene = scene(&pkg);
    let resolved = assets(&pkg, &scene);
    assert!(resolved.draws.is_empty());
    assert_eq!(resolved.skipped_nodes, 1);
    let meta = analyse_scene_with_package(&scene, &pkg).unwrap();
    assert!(!meta.has_sprite_animation);
    let report = compute_compatibility(&meta);
    assert!(
        report
            .unsupported_features
            .iter()
            .any(|error| error.contains("materials/atlas.tex")
                && error.contains("positive finite frame durations"))
    );
}
