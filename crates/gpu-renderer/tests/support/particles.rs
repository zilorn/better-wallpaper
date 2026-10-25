use super::*;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/particles")
}
fn assets(name: &str) -> Scene2dAssets {
    let pkg =
        PkgReader::parse(std::fs::read(root().join(name).join("scene.pkg")).unwrap()).unwrap();
    let graph = parse_scene_graph(
        &pkg.read_entry_string(pkg.find("scene.json").unwrap())
            .unwrap(),
    )
    .unwrap();
    resolve_scene_2d_assets(
        &pkg,
        build_scene_2d_plan(
            &graph,
            Scene2dOptions {
                viewport_width: 128,
                viewport_height: 128,
            },
        )
        .unwrap(),
    )
    .unwrap()
}
fn native(name: &str) -> Vec<u8> {
    let mut decoder = png::Decoder::new(
        std::fs::File::open(root().join("captures").join(format!("{name}.png"))).unwrap(),
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).unwrap();
    assert_eq!((info.width, info.height), (128, 128));
    match info.color_type {
        png::ColorType::Rgb => data[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => data[..info.buffer_size()]
            .iter()
            .flat_map(|&c| [c, c, c, 255])
            .collect(),
        other => panic!("unsupported native PNG color type {other:?}"),
    }
}

#[test]
fn owned_particle_inputs_and_native_captures_have_fixed_hashes() {
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
    for (path, hash) in manifest["native_capture_sha256"].as_object().unwrap() {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(root().join("captures").join(path)).unwrap())
            ),
            hash.as_str().unwrap(),
            "{path}"
        );
    }
    for (name, center) in [
        ("static", [64, 64, 8, 255]),
        ("overlap", [96, 96, 12, 255]),
        ("additive", [128, 128, 16, 255]),
    ] {
        let image = native(name);
        assert_eq!(
            &image[(64 * 128 + 64) * 4..(64 * 128 + 64) * 4 + 4],
            &center
        );
    }
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; particle native pixels, budgets, pause and resource lifecycle"]
fn particle_batches_match_native_and_survive_clock_and_context_changes() {
    let mut comparisons = 0;
    let mut maximum_error = 0;
    for _ in 0..2 {
        let context = Offscreen::with_size(128);
        let mut renderer = SceneGpuRenderer::new(context.gl(), 128, 128).unwrap();
        for _ in 0..2 {
            for name in [
                "static",
                "overlap",
                "additive",
                "scale-one",
                "scale-two",
                "pattern",
                "rotated",
                "stretched",
                "finite",
            ] {
                let mut a = assets(name);
                assert_eq!(a.draws.len(), 1);
                assert_eq!(a.skipped_nodes, 0);
                renderer.clear_textures();
                renderer.upload_scene_textures(&a).unwrap();
                let times = if name == "finite" {
                    [2., 3., 3., 100., 2.]
                } else {
                    [0., 1., 1., 100., 0.]
                };
                for time in times {
                    renderer.draw_scene(&a, time).unwrap();
                    let actual = context.pixels(128, 128);
                    let expected = native(name);
                    let max = actual
                        .iter()
                        .zip(&expected)
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap();
                    maximum_error = maximum_error.max(max);
                    if max > 1 {
                        write_png(
                            &std::path::PathBuf::from(format!("/tmp/a6-{name}-actual.png")),
                            128,
                            128,
                            &actual,
                        );
                    }
                    assert!(max <= 1, "native {name} at {time}: maximum error {max}");
                    comparisons += 1;
                }
                a.set_particle_limit(0);
                renderer.draw_scene(&a, 1.).unwrap();
                assert!(
                    context
                        .pixels(128, 128)
                        .chunks_exact(4)
                        .all(|p| p == [0, 0, 0, 255])
                );
            }
        }
        let mut a = assets("random");
        a.set_particle_limit(7);
        renderer.clear_textures();
        renderer.upload_scene_textures(&a).unwrap();
        renderer.draw_scene(&a, 1.).unwrap();
        let frozen = context.pixels(128, 128);
        for time in [1., 1., 1.1, 20., 1e9, 0., 1.] {
            renderer.draw_scene(&a, time).unwrap();
        }
        assert_eq!(context.pixels(128, 128), frozen);
        renderer.clear_textures();
        renderer.upload_scene_textures(&a).unwrap();
        renderer.draw_scene(&a, 1.).unwrap();
        assert_eq!(context.pixels(128, 128), frozen);
        drop(a);
        renderer.clear_textures();
        let valid = assets("static");
        assert!(
            renderer
                .draw_scene(&valid, 0.)
                .unwrap_err()
                .contains("has not been uploaded")
        );
        let mut broken = valid.clone();
        broken.draws[0].texture.levels[0].data = Arc::from([]);
        assert!(renderer.upload_scene_textures(&broken).is_err());
        renderer.upload_scene_textures(&valid).unwrap();
        renderer.draw_scene(&valid, 0.).unwrap();
        let expected = native("static");
        assert!(
            context
                .pixels(128, 128)
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        renderer.clear_textures();
        // Draw a conventional quad after a colored particle batch. The constant
        // white vertex attribute and 24-byte layout must be restored.
        let normal = super::motion_native::assets("baseline");
        renderer.upload_scene_textures(&normal).unwrap();
        renderer.draw_scene(&normal, 0.).unwrap();
        assert_eq!(
            context.pixels(128, 128),
            super::motion_native::expected("baseline")
        );
    }
    println!(
        "A6 native particle comparisons: {comparisons}; maximum channel error: {maximum_error}"
    );
}
