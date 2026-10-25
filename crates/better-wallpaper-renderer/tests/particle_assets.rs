use better_wallpaper_renderer::{
    Scene2dAssets, Scene2dOptions, build_scene_2d_plan, resolve_scene_2d_assets,
};
use better_wallpaper_scene_format::{PkgReader, parse_scene_graph};
use std::{path::Path, sync::Arc};

fn assets(name: &str, objects: Option<&str>) -> Scene2dAssets {
    let pkg = PkgReader::parse(
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/particles")
                .join(name)
                .join("scene.pkg"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut scene: serde_json::Value = serde_json::from_str(
        &pkg.read_entry_string(pkg.find("scene.json").unwrap())
            .unwrap(),
    )
    .unwrap();
    if let Some(objects) = objects {
        scene["objects"] = serde_json::from_str(objects).unwrap();
    }
    let graph = parse_scene_graph(&scene.to_string()).unwrap();
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

#[test]
fn native_size_aspect_color_and_world_transform_reach_the_vertex_batch() {
    let a = assets("static", None);
    assert_eq!(a.draws.len(), 1);
    assert_eq!(a.skipped_nodes, 0);
    let vertices = a.draws[0].particle_vertices_at(0.).unwrap();
    assert_eq!(vertices.len(), 60);
    assert_eq!(
        &vertices[..10],
        &[
            -0.125,
            0.0625,
            0.,
            0.,
            0.,
            0.,
            128. / 255.,
            1.,
            64. / 255.,
            1.
        ]
    );
    let rotated = assets(
        "static",
        Some(
            r#"[{"id":17,"particle":"particles/probe.json","origin":"32 48 0","scale":"2 2 1","angles":"0 0 1.5707963267948966"}]"#,
        ),
    );
    let v = rotated.draws[0].particle_vertices_at(0.).unwrap();
    assert!((v[0] - -0.625).abs() < 1e-6);
    assert!((v[1] - -0.5).abs() < 1e-6);
}

#[test]
fn global_budget_is_shared_in_draw_order_and_zero_disables_all_systems() {
    let mut a = assets(
        "continuous",
        Some(
            r#"[{"id":17,"particle":"particles/probe.json","origin":"64 64 0"},{"id":18,"particle":"particles/probe.json","origin":"64 64 0"}]"#,
        ),
    );
    assert!(Arc::ptr_eq(
        &a.draws[0].texture.levels[0].data,
        &a.draws[1].texture.levels[0].data
    ));
    a.set_particle_limit(23);
    assert_eq!(a.draws[0].particles.as_ref().unwrap().capacity, 20);
    assert_eq!(a.draws[1].particles.as_ref().unwrap().capacity, 3);
    assert_eq!(
        a.draws
            .iter()
            .map(|d| d.particle_vertices_at(2.).unwrap().len() / 60)
            .sum::<usize>(),
        23
    );
    a.set_particle_limit(0);
    assert!(
        a.draws
            .iter()
            .all(|d| d.particle_vertices_at(2.).unwrap().is_empty())
    );
    a.set_particle_limit(u32::MAX);
    assert_eq!(a.draws[1].particles.as_ref().unwrap().capacity, 20);
}

#[test]
fn snapshots_freeze_rewind_reload_and_release_cpu_ownership() {
    let mut a = assets("random", None);
    a.set_particle_limit(7);
    let initial = a.draws[0].particle_vertices_at(1.).unwrap();
    assert_eq!(initial, a.draws[0].particle_vertices_at(1.).unwrap());
    assert_ne!(initial, a.draws[0].particle_vertices_at(1.1).unwrap());
    assert!(a.draws[0].particle_vertices_at(1e9).unwrap().len() <= 7 * 60);
    assert_eq!(initial, a.draws[0].particle_vertices_at(1.).unwrap());
    let pixels = Arc::downgrade(&a.draws[0].texture.levels[0].data);
    drop(a);
    assert!(pixels.upgrade().is_none());
    let mut a = assets("random", None);
    a.set_particle_limit(7);
    assert_eq!(initial, a.draws[0].particle_vertices_at(1.).unwrap());
}

#[test]
fn hard_budget_bounds_actual_vertex_storage_and_default_capacity() {
    let mut a = assets("budget", None);
    assert_eq!(a.draws[0].particles.as_ref().unwrap().capacity, 10_000);
    a.set_particle_limit(100_000);
    let vertices = a.draws[0].particle_vertices_at(0.).unwrap();
    assert_eq!(vertices.len(), 100_000 * 60);
    assert_eq!(vertices.len() * std::mem::size_of::<f32>(), 24_000_000);
    a.set_particle_limit(0);
    assert!(a.draws[0].particle_vertices_at(0.).unwrap().is_empty());
}

#[test]
fn unsupported_particle_keeps_other_draws_and_system_count_is_bounded() {
    let a = assets("unsupported", None);
    assert_eq!(a.draws.len(), 0);
    assert_eq!(a.skipped_nodes, 1);
    let objects=serde_json::to_string(&(0..65).map(|id| serde_json::json!({"id":id,"particle":"particles/probe.json","origin":"64 64 0"})).collect::<Vec<_>>()).unwrap();
    let a = assets("static", Some(&objects));
    assert_eq!(a.draws.len(), 64);
    assert_eq!(a.skipped_nodes, 1);
    let a = assets(
        "static",
        Some(
            r#"[{"id":1,"particle":"particles/probe.json","instanceoverride":{}},{"id":2,"particle":"particles/probe.json"}]"#,
        ),
    );
    assert_eq!(a.draws.len(), 1);
    assert_eq!(a.skipped_nodes, 1);
    let a = assets(
        "static",
        Some(
            r#"[{"id":1,"particle":"particles/probe.json","visible":false,"instanceoverride":{}}]"#,
        ),
    );
    assert!(a.draws.is_empty());
    assert_eq!(a.skipped_nodes, 0);
}
