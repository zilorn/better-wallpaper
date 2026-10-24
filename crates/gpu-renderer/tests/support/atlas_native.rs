//! Original-program captures of independently authored constant-frame probes.
use super::*;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/atlas-native")
}

fn assets(name: &str) -> Scene2dAssets {
    let pkg =
        PkgReader::parse(std::fs::read(root().join(name).join("scene.pkg")).unwrap()).unwrap();
    let graph = parse_scene_graph(
        &pkg.read_entry_string(pkg.find("scene.json").unwrap())
            .unwrap(),
    )
    .unwrap();
    let plan = build_scene_2d_plan(
        &graph,
        Scene2dOptions {
            viewport_width: 32,
            viewport_height: 32,
        },
    )
    .unwrap();
    resolve_scene_2d_assets(&pkg, plan).unwrap()
}

fn compare(context: &Offscreen, name: &str) {
    let actual = context.pixels(128, 128);
    let mut decoder = png::Decoder::new(
        std::fs::File::open(root().join("captures").join(format!("{name}.png"))).unwrap(),
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (128, 128));
    assert_eq!(info.color_type, png::ColorType::Rgb);
    let expected: Vec<_> = pixels[..info.buffer_size()]
        .chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    let score = metrics(&actual, &expected, 128, 128);
    println!("Native {name}: {score:?}");
    if !passes(&score) || score.mse > 0.75 {
        let failures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scene-golden-failures");
        std::fs::create_dir_all(&failures).unwrap();
        write_png(
            &failures.join(format!("native-{name}-actual.png")),
            128,
            128,
            &actual,
        );
        panic!("native comparison failed: {score:?}");
    }
}

#[test]
fn native_capture_inputs_and_references_match_recorded_hashes() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let files = manifest["sha256"].as_object().unwrap();
    assert_eq!(files.len(), 52);
    for (path, hash) in files {
        let bytes = std::fs::read(root().join(path)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            hash.as_str().unwrap(),
            "{path}"
        );
    }
    for name in ["multi-image", "rotated", "flipped-x", "flipped-y", "skewed"] {
        let assets = assets(&format!("{name}-animated"));
        assert_eq!(assets.draws.len(), 1);
        let animation = assets.draws[0].animation.as_ref().unwrap();
        assert!(animation.native_sampling.is_some());
        assert_eq!(animation.frames.len(), 2);
    }
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; compares native Wallpaper Engine captures"]
fn native_atlas_captures() {
    let context = Offscreen::with_size(128);
    for name in [
        "clamped",
        "nearest",
        "packed-mips",
        "packed-point",
        "flipped-x0",
        "flipped-x1",
        "flipped-y0",
        "flipped-y1",
        "rotated0",
        "rotated1",
        "skewed0",
        "skewed1",
        "multi-image0",
        "multi-image1",
    ] {
        let assets = assets(name);
        let mut renderer = SceneGpuRenderer::new(context.gl(), 128, 128).unwrap();
        renderer.upload_scene_textures(&assets).unwrap();
        renderer.draw_scene(&assets, 0.).unwrap();
        compare(&context, name);
    }
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; checks native frame selection and lifecycle"]
fn native_atlas_loop_pause_reload_and_context_recreation() {
    for name in ["multi-image", "rotated", "flipped-x", "flipped-y", "skewed"] {
        let assets = assets(&format!("{name}-animated"));
        for _ in 0..2 {
            let context = Offscreen::with_size(128);
            let mut renderer = SceneGpuRenderer::new(context.gl(), 128, 128).unwrap();
            for reload in 0..2 {
                if reload != 0 {
                    renderer.clear_textures();
                }
                renderer.upload_scene_textures(&assets).unwrap();
                for (time, frame) in [
                    (0., 0),
                    (0.499999, 0),
                    (0.5, 1),
                    (0.5, 1),
                    (1., 0),
                    (0.5, 1),
                ] {
                    renderer.draw_scene(&assets, time).unwrap();
                    compare(&context, &format!("{name}{frame}"));
                }
            }
        }
    }
}
