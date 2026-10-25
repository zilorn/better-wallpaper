//! Opt-in particle presentation check on one explicitly selected real output.
use better_wallpaper_renderer::{Scene2dOptions, build_scene_2d_plan, resolve_scene_2d_assets};
use better_wallpaper_scene_format::{PkgReader, parse_scene_graph};
use better_wallpaper_wayland::NiriBackend;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires live niri and BETTER_WALLPAPER_NIRI_TEST_OUTPUT"]
fn particles_present_with_budget_pause_reload_and_context_replacement() {
    let output =
        std::env::var("BETTER_WALLPAPER_NIRI_TEST_OUTPUT").expect("explicit output required");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/particles");
    let mut backend = NiriBackend::connect(Some(&output)).unwrap();
    let mut submissions = 0;
    for name in ["static", "continuous", "random"] {
        let pkg =
            PkgReader::parse(std::fs::read(root.join(name).join("scene.pkg")).unwrap()).unwrap();
        let graph = parse_scene_graph(
            &pkg.read_entry_string(pkg.find("scene.json").unwrap())
                .unwrap(),
        )
        .unwrap();
        let mut assets = resolve_scene_2d_assets(
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
        for (lifecycle, budget) in [(0, 7), (1, 0), (2, 7)] {
            if lifecycle == 2 {
                drop(backend);
                backend = NiriBackend::connect(Some(&output)).unwrap();
            }
            assets.set_particle_limit(budget);
            backend.load_scene_assets(assets.clone()).unwrap();
            for time in [0., 0.5, 0.5, 1., 1e9, 0.] {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !backend.present_scene(time).unwrap() {
                    assert!(
                        Instant::now() < deadline,
                        "{name}: no particle submission on {output}"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                submissions += 1;
            }
            assert_eq!(backend.output_name(), output);
            println!(
                "A6 {name}: lifecycle {lifecycle}, budget {budget}, size {:?}",
                backend.size()
            );
        }
    }
    println!("A6 live particle submissions: {submissions}");
}
