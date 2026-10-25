//! Opt-in live A4 presentation check. Creates only temporary background layers.
use better_wallpaper_renderer::{Scene2dOptions, build_scene_2d_plan, resolve_scene_2d_assets};
use better_wallpaper_scene_format::{PkgReader, parse_scene_graph};
use better_wallpaper_wayland::NiriBackend;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires live niri and BETTER_WALLPAPER_NIRI_TEST_OUTPUT"]
fn effects_and_motion_present_across_pause_reload_and_context_replacement() {
    let output =
        std::env::var("BETTER_WALLPAPER_NIRI_TEST_OUTPUT").expect("explicit test output required");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut backend = NiriBackend::connect(Some(&output)).unwrap();
    let mut submissions = 0;
    for (fixture, name) in [
        ("tint-native", "mask"),
        ("tint-native", "animated"),
        ("motion-native", "origin"),
        ("motion-native", "scale-relative"),
        ("motion-native", "parallax-aspect"),
        ("motion-native", "parallax-smooth"),
    ] {
        let pkg = PkgReader::parse(
            std::fs::read(root.join(fixture).join(name).join("scene.pkg")).unwrap(),
        )
        .unwrap();
        let graph = parse_scene_graph(
            &pkg.read_entry_string(pkg.find("scene.json").unwrap())
                .unwrap(),
        )
        .unwrap();
        let assets = resolve_scene_2d_assets(
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
        .unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(assets.skipped_nodes, 0);
        for lifecycle in 0..3 {
            if lifecycle == 2 {
                drop(backend);
                backend = NiriBackend::connect(Some(&output)).unwrap();
            }
            backend.load_scene_assets(assets.clone()).unwrap();
            for time in [0., 0.25, 0.5, 0.5, 1., 0.] {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !backend.present_scene(time).unwrap() {
                    assert!(
                        Instant::now() < deadline,
                        "{name}: no live submission on {output}"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                submissions += 1;
            }
            assert_eq!(backend.output_name(), output);
            println!(
                "A4 {name}: lifecycle {lifecycle}, 6 submissions on {output} at {:?}",
                backend.size()
            );
        }
    }
    println!("A4 live submissions: {submissions}");
}
