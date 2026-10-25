use super::*;

const NAMES: [&str; 11] = [
    "baseline", "tint", "bypass", "partial", "multiply", "screen", "normal", "alpha", "mask",
    "disabled", "animated",
];

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tint-native")
}

fn assets(name: &str) -> Scene2dAssets {
    let pkg =
        PkgReader::parse(std::fs::read(root().join(name).join("scene.pkg")).unwrap()).unwrap();
    let json = pkg
        .read_entry_string(pkg.find("scene.json").unwrap())
        .unwrap();
    let graph = parse_scene_graph(&json).unwrap();
    let plan = build_scene_2d_plan(
        &graph,
        Scene2dOptions {
            viewport_width: 128,
            viewport_height: 128,
        },
    )
    .unwrap();
    let assets = resolve_scene_2d_assets(&pkg, plan).unwrap();
    assert_eq!(assets.draws.len(), 1, "{name}");
    assert_eq!(assets.skipped_nodes, 0, "{name}");
    assets
}

fn expected(name: &str) -> Vec<u8> {
    let mut decoder = png::Decoder::new(
        std::fs::File::open(root().join("captures").join(format!("{name}.png"))).unwrap(),
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (128, 128));
    assert_eq!(info.color_type, png::ColorType::Rgb);
    pixels[..info.buffer_size()]
        .chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}

#[test]
fn tint_inputs_and_native_capture_identities() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for (path, hash) in manifest["sha256"].as_object().unwrap() {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(root().join(path)).unwrap())
            ),
            hash.as_str().unwrap(),
            "{path}"
        );
    }
    for name in NAMES {
        assert_eq!(
            assets(name).draws[0].tint.is_some(),
            !matches!(name, "baseline" | "disabled")
        );
    }
    assert_eq!(expected("baseline"), expected("bypass"));
    assert_eq!(expected("baseline"), expected("disabled"));
    assert!(passes(&metrics(
        &expected("tint"),
        &expected("animated"),
        128,
        128
    )));
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; original-program Tint references"]
fn tint_matches_native_blending_masks_alpha_and_lifecycle() {
    let cases: Vec<_> = NAMES
        .into_iter()
        .map(|name| (name, assets(name), expected(name)))
        .collect();
    let mut comparisons = 0;
    for _ in 0..2 {
        let context = Offscreen::with_size(128);
        let mut renderer = SceneGpuRenderer::new(context.gl(), 128, 128).unwrap();
        for _ in 0..2 {
            for (name, assets, expected) in &cases {
                renderer.clear_textures();
                renderer.upload_scene_textures(assets).unwrap();
                for time in [2., 2., 8., 2.] {
                    renderer.draw_scene(assets, time).unwrap();
                    let actual = context.pixels(128, 128);
                    let comparison = metrics(&actual, expected, 128, 128);
                    println!("Tint {name} t={time}: {comparison:?}");
                    if !passes(&comparison) {
                        write_png(
                            &std::path::PathBuf::from(format!("/tmp/a4-tint-{name}-actual.png")),
                            128,
                            128,
                            &actual,
                        );
                        panic!("Tint {name}: {comparison:?}");
                    }
                    comparisons += 1;
                }
            }
        }
    }
    println!("Tint native comparisons: {comparisons}");
}
