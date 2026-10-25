//! Opt-in live A7 probe; creates temporary background surfaces only.
use better_wallpaper_renderer::{Scene2dOptions, build_scene_2d_plan, resolve_scene_2d_assets};
use better_wallpaper_scene_format::{PkgReader, parse_scene_graph_with_scripts};
use better_wallpaper_wayland::NiriBackend;
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires live niri and BETTER_WALLPAPER_NIRI_TEST_OUTPUT"]
fn restricted_scripts_present_across_pause_reload_fault_and_context_replacement() {
    let output =
        std::env::var("BETTER_WALLPAPER_NIRI_TEST_OUTPUT").expect("explicit test output required");
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/scenescript-native");
    let mut backend = NiriBackend::connect(Some(&output)).unwrap();
    let props = BTreeMap::from([("opacity".into(), "0.5".into())]);
    let mut submissions = 0;
    for name in ["origin", "scale", "property", "motion", "fault"] {
        for lifecycle in 0..3 {
            let pkg = PkgReader::parse(std::fs::read(root.join(name).join("scene.pkg")).unwrap())
                .unwrap();
            let source = pkg
                .read_entry_string(pkg.find("scene.json").unwrap())
                .unwrap();
            let graph = parse_scene_graph_with_scripts(&source, &props, &props, true).unwrap();
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
            if lifecycle == 2 {
                drop(backend);
                backend = NiriBackend::connect(Some(&output)).unwrap();
            }
            backend.load_scene_assets(assets).unwrap();
            for time in [0., 0.25, 0.5, 0.5, 1., 0.] {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !backend.present_scene(time).unwrap() {
                    assert!(Instant::now() < deadline, "{name}: no live submission");
                    std::thread::sleep(Duration::from_millis(5));
                }
                submissions += 1;
            }
            println!(
                "A7 {name}: lifecycle {lifecycle}, 6 submissions on {output}, {:?}",
                backend.size()
            );
        }
    }
    println!("A7 live submissions: {submissions}");
}
