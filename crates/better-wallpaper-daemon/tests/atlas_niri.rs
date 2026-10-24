//! Opt-in live desktop submission test; temporarily creates a background layer.
use better_wallpaper_renderer::{Scene2dOptions, build_scene_2d_plan, resolve_scene_2d_assets};
use better_wallpaper_scene_format::{PkgReader, parse_scene_graph};
use better_wallpaper_wayland::NiriBackend;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires a live niri desktop and BETTER_WALLPAPER_NIRI_TEST_OUTPUT"]
fn native_atlases_present_after_pause_reload_and_context_recreation() {
    let output =
        std::env::var("BETTER_WALLPAPER_NIRI_TEST_OUTPUT").expect("explicit test output required");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/atlas-native");
    let mut backend = NiriBackend::connect(Some(&output)).unwrap();
    for name in ["multi-image", "rotated", "flipped-x", "flipped-y", "skewed"] {
        let pkg = PkgReader::parse(
            std::fs::read(root.join(format!("{name}-animated/scene.pkg"))).unwrap(),
        )
        .unwrap();
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
        let assets = resolve_scene_2d_assets(&pkg, plan).unwrap();
        for lifecycle in 0..3 {
            if lifecycle == 2 {
                drop(backend);
                backend = NiriBackend::connect(Some(&output)).unwrap();
            }
            backend.load_scene_assets(assets.clone()).unwrap();
            for time in [0., 0.499999, 0.5, 0.5, 1., 0.5] {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !backend.present_scene(time).unwrap() {
                    assert!(
                        Instant::now() < deadline,
                        "{name}: no live submission on {output}"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            assert_eq!(backend.output_name(), output);
            println!(
                "{name}: lifecycle {lifecycle}, 6 submissions on {output} at {:?}",
                backend.size()
            );
        }
    }
}
