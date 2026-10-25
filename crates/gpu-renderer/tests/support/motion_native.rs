use super::*;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/motion-native")
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
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
    assert_eq!(assets.draws.len(), 1);
    assert_eq!(assets.skipped_nodes, 0);
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
fn motion_capture_hashes_and_native_smoothing_trajectory() {
    let manifest = manifest();
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
    let trajectory: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root().join("trajectory.json")).unwrap()).unwrap();
    let mut centers = Vec::new();
    for row in trajectory {
        let center = row["center_x"].as_f64().unwrap();
        if centers.last() != Some(&center) {
            centers.push(center);
        }
    }
    assert_eq!(centers.len(), 5); // Initial position and four visible native steps.
    let settings = better_wallpaper_scene_format::SceneParallax {
        amount: 0.5,
        delay: 0.25,
        mouse_influence: 1.,
    };
    let mut state = better_wallpaper_renderer::SceneParallaxState::default();
    state.update(Some(settings), 0., Some([0., 0.]));
    // Native window presents at approximately 15 Hz; capture timestamps bracket
    // readback, not engine update time. Compare per-update rasterized centers.
    for (frame, center) in centers.into_iter().enumerate() {
        let pointer = state.update(Some(settings), frame as f64 / 15., Some([0.5, 0.]));
        let predicted = 63.5 - f64::from(pointer[0]) * 0.5 * 64.;
        assert!(
            (predicted - center).abs() <= 0.5,
            "frame {frame}: {predicted} versus {center}"
        );
    }
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; native motion, parallax and analytical trajectories"]
fn motion_and_parallax_match_native_endpoints_and_scene_clock() {
    let manifest = manifest();
    let mut names: Vec<_> = manifest["sha256"]
        .as_object()
        .unwrap()
        .keys()
        .filter_map(|p| {
            p.strip_prefix("captures/")
                .and_then(|p| p.strip_suffix(".png"))
        })
        .collect();
    names.sort();
    let mut comparisons = 0;
    for _ in 0..2 {
        let context = Offscreen::with_size(128);
        let mut renderer = SceneGpuRenderer::new(context.gl(), 128, 128).unwrap();
        for _ in 0..2 {
            for name in &names {
                let assets = assets(name);
                let reference = expected(name);
                let pointer = manifest["pointer"][name].as_array().map(|p| {
                    [
                        p[0].as_f64().unwrap() as f32 / 64. - 1.,
                        1. - p[1].as_f64().unwrap() as f32 / 64.,
                    ]
                });
                renderer.clear_textures();
                renderer.upload_scene_textures(&assets).unwrap();
                renderer.draw_scene_with_pointer(&assets, 0., None).unwrap();
                for time in [2., 2., 8.] {
                    renderer
                        .draw_scene_with_pointer(&assets, time, pointer)
                        .unwrap();
                    let actual = context.pixels(128, 128);
                    let score = metrics(&actual, &reference, 128, 128);
                    println!("Native motion {name} t={time}: {score:?}");
                    if !passes(&score) {
                        write_png(
                            &std::path::PathBuf::from(format!("/tmp/a4-motion-{name}-actual.png")),
                            128,
                            128,
                            &actual,
                        );
                        panic!("Native motion {name}: {score:?}");
                    }
                    comparisons += 1;
                }
            }
        }
        let moving = assets("origin");
        renderer.clear_textures();
        renderer.upload_scene_textures(&moving).unwrap();
        for time in [0., 0.25, 0.5, 0.5, 0.75, 1., 0.] {
            renderer.draw_scene(&moving, time).unwrap();
            // Independent rectangle geometry: X 48->80, Y 64->48; authored
            // position channels add to the saved origin (zero in this probe).
            let cx = 48. + 32. * time;
            let cy = 64. - 16. * time;
            let reference: Vec<_> = (0..128)
                .flat_map(|y| {
                    (0..128).flat_map(move |x| {
                        if ((x as f64 + 0.5) - cx).abs() < 8.
                            && ((127.5 - y as f64) - cy).abs() < 8.
                        {
                            [255, 64, 16, 255]
                        } else {
                            [0, 0, 0, 255]
                        }
                    })
                })
                .collect();
            let score = metrics(&context.pixels(128, 128), &reference, 128, 128);
            assert!(passes(&score), "trajectory {time}: {score:?}");
            comparisons += 1;
        }
    }
    println!("A4 motion/parallax comparisons: {comparisons}");
}
