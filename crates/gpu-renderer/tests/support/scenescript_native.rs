use super::*;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/scenescript-native")
}

fn assets(name: &str, enabled: bool, opacity: &str) -> Scene2dAssets {
    let pkg =
        PkgReader::parse(std::fs::read(root().join(name).join("scene.pkg")).unwrap()).unwrap();
    let source = pkg
        .read_entry_string(pkg.find("scene.json").unwrap())
        .unwrap();
    let props = BTreeMap::from([("opacity".into(), opacity.into())]);
    let graph = better_wallpaper_scene_format::parse_scene_graph_with_scripts(
        &source, &props, &props, enabled,
    )
    .unwrap();
    let plan = build_scene_2d_plan(
        &graph,
        Scene2dOptions {
            viewport_width: 128,
            viewport_height: 128,
        },
    )
    .unwrap();
    let assets = resolve_scene_2d_assets(&pkg, plan).unwrap();
    assert_eq!(assets.draws.len(), 1);
    assert_eq!(assets.skipped_nodes, 0);
    assets
}

fn native(name: &str) -> Vec<u8> {
    let mut decoder = png::Decoder::new(
        std::fs::File::open(root().join("captures").join(format!("{name}.png"))).unwrap(),
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(
        (info.width, info.height, info.color_type),
        (128, 128, png::ColorType::Rgb)
    );
    bytes[..info.buffer_size()]
        .chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}

fn marker(center: [f32; 2], rgb: [u8; 3]) -> Vec<u8> {
    let mut image = [0, 0, 0, 255].repeat(128 * 128);
    for y in 0..128 {
        for x in 0..128 {
            if (x as f32 + 0.5 - center[0]).abs() < 8. && (y as f32 + 0.5 - center[1]).abs() < 8. {
                image[(y * 128 + x) * 4..(y * 128 + x) * 4 + 3].copy_from_slice(&rgb);
            }
        }
    }
    image
}

#[test]
fn script_inputs_native_hashes_and_stateful_rejection_are_pinned() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for (path, digest) in manifest["sha256"].as_object().unwrap() {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(root().join(path)).unwrap())
            ),
            digest.as_str().unwrap(),
            "{path}"
        );
    }
    let pkg =
        PkgReader::parse(std::fs::read(root().join("base-value/scene.pkg")).unwrap()).unwrap();
    let source = pkg
        .read_entry_string(pkg.find("scene.json").unwrap())
        .unwrap();
    let graph = better_wallpaper_scene_format::parse_scene_graph_with_scripts(
        &source,
        &Default::default(),
        &Default::default(),
        true,
    )
    .unwrap();
    assert!(graph.nodes[0].transform.scripts.is_empty());
    assert!(
        graph
            .unsupported_features
            .iter()
            .any(|f| f.feature.contains("stateful previous-value"))
    );
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; native SceneScript and bounded lifecycle comparisons"]
fn restricted_scripts_match_native_and_survive_pause_fault_and_reload() {
    let mut comparisons = 0;
    for _ in 0..2 {
        let context = Offscreen::with_size(128);
        let mut renderer = SceneGpuRenderer::new(context.gl(), 128, 128).unwrap();
        for _ in 0..2 {
            for name in [
                "origin",
                "scale",
                "alpha",
                "layer",
                "scene-layer",
                "property",
                "clock",
            ] {
                let assets = assets(name, true, "0.5");
                let expected = native(name);
                renderer.clear_textures();
                renderer.upload_scene_textures(&assets).unwrap();
                for time in [0., 0.5, 0.5, 1., 0.] {
                    renderer.draw_scene(&assets, time).unwrap();
                    let actual = context.pixels(128, 128);
                    let score = metrics(&actual, &expected, 128, 128);
                    assert!(passes(&score), "{name}@{time}: {score:?}");
                    comparisons += 1;
                }
            }
            let motion = assets("motion", true, "0.5");
            renderer.clear_textures();
            renderer.upload_scene_textures(&motion).unwrap();
            for time in [0., 0.25, 0.5, 0.5, 1., 0.] {
                renderer.draw_scene(&motion, time).unwrap();
                assert_eq!(
                    context.pixels(128, 128),
                    marker([64. + time as f32 * 16., 64.], [255, 64, 16])
                );
                comparisons += 1;
            }
            let disabled = assets("origin", false, "0.5");
            renderer.clear_textures();
            renderer.upload_scene_textures(&disabled).unwrap();
            renderer.draw_scene(&disabled, 1.).unwrap();
            assert_eq!(context.pixels(128, 128), marker([64., 64.], [255, 64, 16]));
            comparisons += 1;
            let override_property = assets("property", true, "0.25");
            renderer.clear_textures();
            renderer.upload_scene_textures(&override_property).unwrap();
            renderer.draw_scene(&override_property, 0.).unwrap();
            assert!(passes(&metrics(
                &context.pixels(128, 128),
                &marker([64., 64.], [64, 16, 4]),
                128,
                128
            )));
            comparisons += 1;
            let fault = assets("fault", true, "0.5");
            renderer.clear_textures();
            renderer.upload_scene_textures(&fault).unwrap();
            for time in [0.5, 1.] {
                // First call faults; saved alpha stays until reload.
                renderer.draw_scene(&fault, time).unwrap();
                assert_eq!(context.pixels(128, 128), marker([64., 64.], [255, 64, 16]));
                comparisons += 1;
            }
            let reloaded = assets("fault", true, "0.5");
            renderer.clear_textures();
            renderer.upload_scene_textures(&reloaded).unwrap();
            renderer.draw_scene(&reloaded, 1.5).unwrap();
            assert!(passes(&metrics(
                &context.pixels(128, 128),
                &marker([64., 64.], [128, 32, 8]),
                128,
                128
            )));
            comparisons += 1;
        }
    }
    println!("A7 native/analytic/lifecycle comparisons: {comparisons}");
}
