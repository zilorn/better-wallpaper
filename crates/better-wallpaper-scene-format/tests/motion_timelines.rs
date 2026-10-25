use better_wallpaper_scene_format::{
    AnimationMode, ScalarAnimation, ScalarKeyframe, analyse_scene, compute_compatibility,
    parse_scene_graph, parse_scene_graph_with_properties,
};
use serde_json::json;
use std::collections::BTreeMap;

fn input() -> serde_json::Value {
    json!({"objects":[{"image":"models/a.json","origin":{"user":"position","value":"64 32 7","animation":{"relative":false,"c0":[{"frame":0,"value":-16},{"frame":30,"value":16}],"c1":[],"c2":[],"options":{"fps":30,"length":30,"mode":"loop","wraploop":false}}}}]})
}

#[test]
fn linear_transform_channels_are_offsets_and_empty_axes_retain_saved_values() {
    let value = input();
    let graph = parse_scene_graph_with_properties(
        &value.to_string(),
        &BTreeMap::from([("position".into(), "80 40 9".into())]),
    )
    .unwrap();
    let transform = &graph.nodes[0].transform;
    let animation = transform.origin_animation.as_ref().unwrap();
    for (time, x) in [(0., 64.), (0.5, 80.), (1., 64.), (1.5, 80.)] {
        let v = animation.value_at(transform.origin.unwrap(), time);
        assert_eq!([v.x, v.y, v.z], [x, 40., 9.]);
    }
    let meta = analyse_scene(&value.to_string()).unwrap();
    assert!(meta.has_timeline_animation);
    assert!(compute_compatibility(&meta).unsupported_features.is_empty());
}

#[test]
fn unverified_curves_scripts_and_schedules_retain_fallback_and_specific_reports() {
    for kind in [
        "curve",
        "script",
        "schedule",
        "wrap",
        "duplicate",
        "range",
        "budget",
        "z",
    ] {
        let mut value = input();
        let origin = &mut value["objects"][0]["origin"];
        match kind {
            "curve" => origin["animation"]["c0"][0]["front"] = json!({"enabled":true,"x":1,"y":0}),
            "script" => origin["script"] = json!("return value"),
            "schedule" => origin["animation"]["options"]["startpaused"] = json!(true),
            "wrap" => origin["animation"]["options"]["wraploop"] = json!(true),
            "duplicate" => origin["animation"]["c0"][1]["frame"] = json!(0),
            "range" => origin["animation"]["c0"][1]["frame"] = json!(31),
            "z" => origin["animation"]["c2"] = json!([{"frame":0,"value":1}]),
            _ => origin["animation"]["c0"] = json!(vec![json!({"frame":0,"value":1}); 1025]),
        }
        let graph = parse_scene_graph(&value.to_string()).unwrap();
        assert!(
            graph.nodes[0].transform.origin_animation.is_none(),
            "{kind}"
        );
        assert_eq!(graph.nodes[0].transform.origin.unwrap().x, 64.);
        let report = compute_compatibility(&analyse_scene(&value.to_string()).unwrap());
        assert!(
            report
                .unsupported_features
                .iter()
                .any(|f| f.contains("timeline")),
            "{kind}"
        );
    }
}

#[test]
fn scalar_clock_preserves_fractional_frames_after_long_uptime() {
    let animation = ScalarAnimation {
        keyframes: vec![
            ScalarKeyframe {
                frame: 0.,
                value: 0.,
            },
            ScalarKeyframe {
                frame: 30.,
                value: 1.,
            },
        ],
        fps: 30.,
        length_frames: 30.,
        mode: AnimationMode::Loop,
    };
    assert_eq!(animation.value_at(16_777_216.25), 0.25);
    assert!(animation.value_at(f64::MAX).is_finite());
    assert_eq!(animation.value_at(f64::NAN), 0.);
    assert_eq!(animation.value_at(-1.), 0.);
}
