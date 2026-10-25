//! Owned inputs rendered with the original ccsimple fragment shader.
use super::*;

const NAMES: [&str; 11] = [
    "baseline",
    "bypass",
    "neutral",
    "invert",
    "mixed",
    "nonlinear",
    "raw",
    "nearest",
    "repeat",
    "alpha",
    "noncubic",
];

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lut-native")
}

fn package(name: &str) -> PkgReader {
    PkgReader::parse(std::fs::read(root().join(name).join("scene.pkg")).unwrap()).unwrap()
}

fn assets(name: &str) -> Scene2dAssets {
    let pkg = package(name);
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
    assert_eq!(assets.skipped_nodes, 0, "{name}");
    assert_eq!(assets.draws.len(), 1, "{name}");
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
fn lut_capture_hashes_and_package_admission() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["sha256"].as_object().unwrap().len(), 33);
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
        let assets = assets(name);
        assert_eq!(assets.draws[0].color_lut.is_some(), name != "baseline");
        let pkg = package(name);
        let json = pkg
            .read_entry_string(pkg.find("scene.json").unwrap())
            .unwrap();
        let meta = better_wallpaper_scene_format::analyse_scene_with_package(&json, &pkg).unwrap();
        assert!(meta.unsupported_textures.is_empty(), "{meta:?}");
        assert_eq!(
            meta.supported_effects.len(),
            usize::from(name != "baseline")
        );
        assert!(meta.unsupported_effects.is_empty());
    }
    assert_eq!(expected("baseline"), expected("bypass"));
    // The original neutral 32^3 cube samples normalized edges, not (N-1)*RGB.
    // Its observed finite-lattice bias is retained rather than hidden by a
    // special-case neutral detector or changed reference pixels.
    let max = expected("baseline")
        .iter()
        .zip(expected("neutral"))
        .map(|(a, b)| a.abs_diff(b))
        .max()
        .unwrap();
    assert_eq!(max, 4);
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; compares original-program LUT sampling"]
fn native_lut_captures_and_lifecycle() {
    for _ in 0..2 {
        let context = Offscreen::with_size(128);
        let mut renderer = SceneGpuRenderer::new(context.gl(), 128, 128).unwrap();
        for reload in 0..2 {
            if reload != 0 {
                renderer.clear_textures();
            }
            for name in NAMES {
                let assets = assets(name);
                renderer.clear_textures();
                renderer.upload_scene_textures(&assets).unwrap();
                renderer.upload_scene_textures(&assets).unwrap();
                for time in [0., 12., 12., 0.] {
                    renderer.draw_scene(&assets, time).unwrap();
                    let actual = context.pixels(128, 128);
                    let score = metrics(&actual, &expected(name), 128, 128);
                    println!("Native LUT {name}: {score:?}");
                    if !passes(&score) || score.mse > 0.75 {
                        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("../../target/scene-golden-failures");
                        std::fs::create_dir_all(&dir).unwrap();
                        write_png(
                            &dir.join(format!("lut-{name}-actual.png")),
                            128,
                            128,
                            &actual,
                        );
                        panic!("Native LUT {name} differs: {score:?}");
                    }
                }
            }
        }
        renderer.clear_textures();
        let mut invalid = assets("nonlinear");
        assert!(
            renderer
                .draw_scene(&invalid, 0.)
                .unwrap_err()
                .contains("has not been uploaded")
        );
        invalid.draws[0].color_lut.as_mut().unwrap().texture.pixels = vec![0; 3].into();
        assert!(
            renderer
                .upload_scene_textures(&invalid)
                .unwrap_err()
                .contains("byte count")
        );
        let valid = assets("nonlinear");
        renderer.upload_scene_textures(&valid).unwrap();
        renderer.draw_scene(&valid, 0.).unwrap();
        assert!(passes(&metrics(
            &context.pixels(128, 128),
            &expected("nonlinear"),
            128,
            128
        )));

        renderer.clear_textures();
        let mut many = valid.clone();
        many.draws = (0..256)
            .map(|index| {
                let mut draw = valid.draws[0].clone();
                draw.color_lut.as_mut().unwrap().path = format!("lut-{index}");
                draw
            })
            .collect();
        assert!(
            renderer
                .upload_scene_textures(&many)
                .unwrap_err()
                .contains("image count exceeds 256")
        );
        // Admission never evicts an earlier active LUT or draws the final layer
        // without grading after the combined image/LUT limit is reached.
        assert!(renderer.draw_scene(&many, 0.).is_err());
        many.draws.truncate(1);
        renderer.draw_scene(&many, 0.).unwrap();
        assert!(passes(&metrics(
            &context.pixels(128, 128),
            &expected("nonlinear"),
            128,
            128
        )));
        if let Ok(installation) = std::env::var("BETTER_WALLPAPER_LUT_INSTALLATION") {
            let manifest: serde_json::Value =
                serde_json::from_str(include_str!("../../../../docs/validation/a2-volumes.json"))
                    .unwrap();
            let selected = manifest["assets"].as_array().unwrap();
            assert_eq!(selected.len(), 28);
            for asset in selected {
                let path = asset["path"].as_str().unwrap();
                let bytes = std::fs::read(Path::new(&installation).join(path)).unwrap();
                assert_eq!(
                    format!("{:x}", Sha256::digest(&bytes)),
                    asset["sha256"].as_str().unwrap()
                );
                let mut assets = valid.clone();
                let lut = assets.draws[0].color_lut.as_mut().unwrap();
                lut.path = path.into();
                lut.texture = better_wallpaper_scene_format::ColorLut::parse(&bytes).unwrap();
                renderer.clear_textures();
                renderer.upload_scene_textures(&assets).unwrap();
                renderer.draw_scene(&assets, 0.).unwrap();
                println!("Selected local LUT parsed/uploaded/drawn: {path}");
            }
        }
    }
}
