use better_wallpaper_scene_format::{
    PkgReader, TintMaskCache, analyse_scene_with_package, compute_compatibility, parse_scene_graph,
    parse_scene_graph_with_properties, resolve_tint,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

fn fixture(name: &str) -> BTreeMap<String, Vec<u8>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/tint-native")
        .join(name)
        .join("scene.pkg");
    let package = PkgReader::parse(std::fs::read(path).unwrap()).unwrap();
    package
        .entries()
        .iter()
        .map(|entry| (entry.filename.clone(), package.read_entry(entry).to_vec()))
        .collect()
}

fn package(entries: BTreeMap<String, Vec<u8>>) -> PkgReader {
    let mut bytes = Vec::new();
    let mut data = Vec::new();
    let string = |out: &mut Vec<u8>, s: &str| {
        out.extend((s.len() as u32).to_le_bytes());
        out.extend(s.as_bytes());
    };
    string(&mut bytes, "PKGV0001");
    bytes.extend((entries.len() as u32).to_le_bytes());
    for (path, payload) in entries {
        string(&mut bytes, &path);
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend((payload.len() as u32).to_le_bytes());
        data.extend(payload);
    }
    bytes.extend(data);
    PkgReader::parse(bytes).unwrap()
}

fn scene(package: &PkgReader) -> String {
    package
        .read_entry_string(package.find("scene.json").unwrap())
        .unwrap()
        .into_owned()
}

#[test]
fn tint_properties_masks_and_partial_reports_use_shared_admission() {
    let pkg = package(fixture("mask"));
    let input = scene(&pkg);
    let properties = BTreeMap::from([
        ("amount".into(), "0.25".into()),
        ("tint".into(), "1 0.5 0.25".into()),
    ]);
    let graph = parse_scene_graph_with_properties(&input, &properties).unwrap();
    let mut cache = TintMaskCache::default();
    let tint = resolve_tint(&pkg, &graph.nodes[0].effect_instances, &mut cache)
        .unwrap()
        .unwrap();
    assert_eq!(tint.amount, 0.25);
    assert_eq!(tint.color, [1., 0.5, 0.25]);
    let mask = tint.mask.unwrap();
    assert_eq!(mask.uv_scale, [0.75, 1.]);
    let again = resolve_tint(&pkg, &graph.nodes[0].effect_instances, &mut cache)
        .unwrap()
        .unwrap()
        .mask
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &mask.image.levels[0].data,
        &again.image.levels[0].data
    ));
    let mut mixed: Value = serde_json::from_str(&input).unwrap();
    mixed["objects"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":2,"particle":"particles/unsupported.json"}));
    let report =
        compute_compatibility(&analyse_scene_with_package(&mixed.to_string(), &pkg).unwrap());
    assert!(
        report
            .supported_features
            .iter()
            .any(|f| f.contains("tint/effect.json"))
    );
    assert!(
        report
            .unsupported_features
            .iter()
            .any(|f| f.contains("Particle"))
    );
}

#[test]
fn tint_rejects_unverified_shaders_stacks_passes_parameters_and_masks() {
    let base = fixture("mask");
    let mutations = [
        ("mode", json!(20)),
        ("alpha", json!(1.1)),
        ("color", json!("NaN 0 1")),
        ("path", json!("../escape")),
        ("missing", json!("missing")),
        ("curve", json!({"value":0.5,"script":"return 1"})),
    ];
    for (kind, value) in mutations {
        let mut entries = base.clone();
        let mut input: Value = serde_json::from_slice(&entries["scene.json"]).unwrap();
        let pass = &mut input["objects"][0]["effects"][0]["passes"][0];
        match kind {
            "mode" => pass["combos"]["BLENDMODE"] = value,
            "path" | "missing" => pass["textures"][1] = value,
            "color" => pass["constantshadervalues"]["color"] = value,
            _ => pass["constantshadervalues"]["alpha"] = value,
        }
        entries.insert("scene.json".into(), serde_json::to_vec(&input).unwrap());
        let pkg = package(entries);
        let graph = parse_scene_graph(&input.to_string()).unwrap();
        assert!(
            resolve_tint(
                &pkg,
                &graph.nodes[0].effect_instances,
                &mut TintMaskCache::default()
            )
            .is_err(),
            "{kind}"
        );
        assert!(
            !analyse_scene_with_package(&input.to_string(), &pkg)
                .unwrap()
                .unsupported_textures
                .is_empty(),
            "{kind}"
        );
    }
    for kind in [
        "shader",
        "multipass",
        "stack",
        "render-target",
        "mask-layout",
    ] {
        let mut entries = base.clone();
        let mut input: Value = serde_json::from_slice(&entries["scene.json"]).unwrap();
        match kind {
            "shader" => {
                entries.insert("shaders/effects/tint.frag".into(), b"custom".to_vec());
            }
            "multipass" => {
                input["objects"][0]["effects"][0]["passes"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({}));
            }
            "stack" => {
                input["objects"][0]["effects"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"file":"effects/unknown/effect.json"}));
            }
            "render-target" => {
                entries.insert(
                    "effects/tint/effect.json".into(),
                    br#"{"fbos":[],"passes":[]}"#.to_vec(),
                );
            }
            _ => {
                entries.get_mut("materials/mask.tex").unwrap()[22..26]
                    .copy_from_slice(&2_u32.to_le_bytes());
            }
        }
        let pkg = package(entries);
        let graph = parse_scene_graph(&input.to_string()).unwrap();
        assert!(
            resolve_tint(
                &pkg,
                &graph.nodes[0].effect_instances,
                &mut TintMaskCache::default()
            )
            .is_err(),
            "{kind}"
        );
    }
}

#[test]
fn mixed_puppet_motion_keeps_supported_tint_and_reports_unsupported_consumer() {
    let mut entries = fixture("tint");
    let mut input: Value = serde_json::from_slice(&entries["scene.json"]).unwrap();
    input["objects"][0]["origin"] = json!({"value":"64 64 0","animation":{"c0":[{"frame":0,"value":1}],"options":{"fps":30,"length":30,"mode":"single"}}});
    entries.insert("scene.json".into(), serde_json::to_vec(&input).unwrap());
    entries.insert(
        "models/probe.json".into(),
        br#"{"material":"materials/probe.json","puppet":"models/probe.mdl"}"#.to_vec(),
    );
    let pkg = package(entries);
    let report =
        compute_compatibility(&analyse_scene_with_package(&input.to_string(), &pkg).unwrap());
    assert!(
        report
            .supported_features
            .iter()
            .any(|feature| feature.contains("tint/effect.json"))
    );
    assert!(
        report
            .unsupported_features
            .iter()
            .any(|feature| feature.contains("puppet transform timelines"))
    );
}
