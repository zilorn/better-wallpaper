use better_wallpaper_scene_format::{
    PkgReader, analyse_scene_with_package, compute_compatibility, parse_scene_graph,
    particle::{ParticleSample, ParticleSystem, resolve_particle, validate_particle_node},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

fn fixture(name: &str) -> PkgReader {
    PkgReader::parse(
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/particles")
                .join(name)
                .join("scene.pkg"),
        )
        .unwrap(),
    )
    .unwrap()
}
fn definition(name: &str) -> Value {
    let p = fixture(name);
    serde_json::from_slice(p.read_entry(p.find("particles/probe.json").unwrap())).unwrap()
}
fn system(v: &Value) -> Result<ParticleSystem, String> {
    ParticleSystem::parse(v.to_string().as_bytes()).map(|(s, _)| s)
}
fn samples(s: &ParticleSystem, time: f64, cap: u32, seed: u64) -> Vec<ParticleSample> {
    let mut out = Vec::new();
    let count = s.sample(time, seed, cap, |p| out.push(p)).unwrap();
    assert_eq!(count, out.len());
    out
}
fn repack(pkg: &PkgReader, edits: &[(&str, Vec<u8>)]) -> PkgReader {
    let mut entries: BTreeMap<String, Vec<u8>> = pkg
        .entries()
        .iter()
        .map(|e| (e.filename.clone(), pkg.read_entry(e).to_vec()))
        .collect();
    for (path, bytes) in edits {
        entries.insert((*path).into(), bytes.clone());
    }
    let mut header = Vec::new();
    let mut data = Vec::new();
    let string = |b: &mut Vec<u8>, s: &str| {
        b.extend((s.len() as u32).to_le_bytes());
        b.extend(s.as_bytes());
    };
    string(&mut header, "PKGV0001");
    header.extend((entries.len() as u32).to_le_bytes());
    for (path, bytes) in entries {
        string(&mut header, &path);
        header.extend((data.len() as u32).to_le_bytes());
        header.extend((bytes.len() as u32).to_le_bytes());
        data.extend(bytes);
    }
    header.extend(data);
    PkgReader::parse(header).unwrap()
}

#[test]
fn continuous_births_deaths_and_linear_motion_use_scene_time() {
    let s = system(&definition("continuous")).unwrap();
    assert_eq!(s.capacity(), 20);
    assert!(samples(&s, 0., 100, 1).is_empty());
    let at = samples(&s, 0.5, 100, 1);
    assert_eq!(at.len(), 5);
    assert_eq!(at[0].id, 1);
    assert_eq!(at[0].position, [3.2, 0.]);
    let at = samples(&s, 2.1, 100, 1);
    assert_eq!(at.len(), 20);
    assert_eq!(at[0].id, 2); // Identity one died exactly at age two.
    assert_eq!(at.last().unwrap().id, 21);
    let mut finite = definition("continuous");
    finite["emitter"][0]["duration"] = json!(1);
    let s = system(&finite).unwrap();
    assert_eq!(samples(&s, 1., 100, 1).len(), 10);
    assert_eq!(samples(&s, 2., 100, 1).len(), 10);
    assert!(samples(&s, 3., 100, 1).is_empty());
    assert!(
        system(&definition("delayed"))
            .unwrap_err()
            .contains("prewarm")
    );
}

#[test]
fn bursts_freeze_rewind_and_expire_without_clock_history() {
    let s = system(&definition("static")).unwrap();
    assert!(samples(&s, -0.001, 100, 7).is_empty());
    let at = samples(&s, 0., 100, 7);
    assert_eq!(at.len(), 1);
    assert_eq!(at[0].size, 32.);
    assert_eq!(at[0].color, [128. / 255., 1., 64. / 255., 1.]);
    assert_eq!(samples(&s, 1800., 100, 7), at);
    assert!(samples(&s, 3600., 100, 7).is_empty());
    assert_eq!(samples(&s, 0., 100, 7), at);
    assert!(s.sample(f64::NAN, 7, 100, |_| {}).is_err());
}

#[test]
fn random_snapshots_are_bounded_repeatable_and_cadence_independent() {
    let s = system(&definition("random")).unwrap();
    let seed = ParticleSystem::seed("17");
    let at = samples(&s, 1., 100, seed);
    for time in [0., 0.01, 0.02, 0.7, 100., 1e9, 0.1] {
        assert!(samples(&s, time, 100, seed).len() <= 20);
    }
    assert_eq!(samples(&s, 1., 100, seed), at);
    assert_ne!(samples(&s, 1., 100, ParticleSystem::seed("18")), at);
    for p in at {
        assert!((-48. ..=48.).contains(&p.position[0]));
        assert!((-24. ..=24.).contains(&p.position[1]));
        assert!((4. ..=16.).contains(&p.size));
        assert!(p.color.iter().all(|c| (0. ..=1.).contains(c)));
    }
    assert!(s.sample(f64::MAX, seed, 100, |_| {}).is_err());
}

#[test]
fn budgets_drop_fixed_births_without_late_resurrection_or_unbounded_work() {
    let s = system(&definition("continuous")).unwrap();
    for step in 0..1000 {
        let time = f64::from(step) / 100.;
        let at = samples(&s, time, 3, 1);
        assert!(at.len() <= 3);
        assert!(at.iter().all(|p| (p.id - 1) % 20 < 3));
        assert_eq!(at, samples(&s, time, 3, 1));
        assert!(samples(&s, time, 0, 1).is_empty());
    }
    assert!(samples(&s, 1e9, 3, 1).len() <= 3);
    let mut v = definition("random");
    v["maxcount"] = json!(2);
    assert_eq!(system(&v).unwrap().capacity(), 2);
}

#[test]
fn unsupported_elements_fields_and_schedules_fail_closed() {
    let base = definition("static");
    for (pointer, value) in [
        ("/operator/0/name", json!("turbulence")),
        ("/renderer/0/name", json!("spritetrail")),
        ("/initializer/0/max", json!(3601)),
        ("/initializer/1/exponent", json!(2)),
        ("/emitter/0/distancemax", json!("0 0 1")),
        ("/emitter/0/directions", json!("1 0 0")),
        ("/emitter/0/flags", json!(1)),
        ("/operator/0/gravity", json!("0 -1 0")),
        ("/animationmode", json!("randomframe")),
        ("/flags", json!(4)),
        ("/controlpoint", json!([{"id":0,"offset":"1 0 0"}])),
        ("/unknown", json!(true)),
        ("/maxcount", json!(100001)),
        ("/maxcount", json!(1.5)),
        ("/emitter/0/periodic", json!(true)),
        ("/renderer/0/orientation", json!(1)),
    ] {
        let mut v = base.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        v.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(key.into(), value);
        assert!(system(&v).is_err(), "{pointer}: {v}");
    }
    let mut v = base.clone();
    v["initializer"][1] = v["initializer"][0].clone();
    assert!(system(&v).is_err());
    let mut v = definition("continuous");
    v["emitter"][0]["rate"] = json!(10000);
    v["initializer"][0]["min"] = json!(3600);
    v["initializer"][0]["max"] = json!(3600);
    assert!(system(&v).is_err());
    assert!(ParticleSystem::parse(&vec![b' '; 128 * 1024 + 1]).is_err());
}

#[test]
fn node_overrides_scripts_parents_and_nonplanar_transforms_are_rejected() {
    for extra in [
        json!({"instanceoverride":{}}),
        json!({"parent":1}),
        json!({"origin":{"value":"0 0 0","script":"return value"}}),
        json!({"angles":"1 0 0"}),
        json!({"origin":"0 0 1"}),
        json!({"color":"1 0 0"}),
        json!({"effects":[]}),
        json!({"scale":{"value":"1 1 1","animation":{}}}),
    ] {
        let mut node = json!({"id":17,"particle":"particles/probe.json"});
        node.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let input = json!({"objects":[node]}).to_string();
        if let Ok(graph) = parse_scene_graph(&input) {
            assert!(validate_particle_node(&graph.nodes[0]).is_err(), "{input}");
        }
    }
    let graph = parse_scene_graph(
        r#"{"objects":[{"particle":"particles/probe.json","instanceoverride":null}]}"#,
    )
    .unwrap();
    validate_particle_node(&graph.nodes[0]).unwrap();
}

#[test]
fn package_admission_preserves_specific_errors_and_mixed_scene_support() {
    let pkg = fixture("static");
    let scene = pkg
        .read_entry_string(pkg.find("scene.json").unwrap())
        .unwrap();
    let meta = analyse_scene_with_package(&scene, &pkg).unwrap();
    assert_eq!(meta.supported_particle_systems, 1);
    let report = compute_compatibility(&meta);
    assert_eq!(
        report.level,
        better_wallpaper_scene_format::CompatibilityLevel::L3
    );
    assert!(report.unsupported_features.is_empty(), "{report:?}");
    let mut mixed: Value = serde_json::from_str(&scene).unwrap();
    mixed["objects"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":18,"particle":"particles/bad.json"}));
    let bad = definition("unsupported").to_string().into_bytes();
    let pkg = repack(&pkg, &[("particles/bad.json", bad)]);
    let report =
        compute_compatibility(&analyse_scene_with_package(&mixed.to_string(), &pkg).unwrap());
    assert_eq!(
        report.level,
        better_wallpaper_scene_format::CompatibilityLevel::L3
    );
    assert!(
        report
            .unsupported_features
            .iter()
            .any(|e| e.contains("turbulence")),
        "{report:?}"
    );
    let hidden = r#"{"objects":[{"id":17,"particle":"particles/probe.json","visible":false,"instanceoverride":{}}]}"#;
    let meta = analyse_scene_with_package(hidden, &pkg).unwrap();
    assert!(!meta.has_particles);
    assert!(compute_compatibility(&meta).unsupported_features.is_empty());
    let mut objects = (0..65)
        .map(|id| json!({"id":id,"particle":"particles/probe.json","instanceoverride":{}}))
        .collect::<Vec<_>>();
    objects.push(json!({"id":100,"particle":"particles/probe.json"}));
    let meta = analyse_scene_with_package(&json!({"objects":objects}).to_string(), &pkg).unwrap();
    assert_eq!(meta.supported_particle_systems, 1);
}

#[test]
fn textures_materials_paths_and_memory_budgets_share_admission() {
    let pkg = fixture("static");
    let asset = resolve_particle(&pkg, "particles/probe.json", 128).unwrap();
    assert_eq!(asset.texture.levels[0].data.len(), 128);
    assert!(asset.sampling.nearest && asset.sampling.clamp);
    assert!(resolve_particle(&pkg, "particles/probe.json", 127).is_err());
    for value in [
        json!({"passes":[{"shader":"custom","textures":["particle"],"blending":"additive"}]}),
        json!({"passes":[{"shader":"genericparticle","textures":["../secret"],"blending":"additive"}]}),
        json!({"passes":[{"shader":"genericparticle","textures":["particle"],"blending":"additive","combos":{"REFRACT":1}}]}),
    ] {
        let p = repack(
            &pkg,
            &[("materials/particle.json", value.to_string().into_bytes())],
        );
        assert!(resolve_particle(&p, "particles/probe.json", 1024).is_err());
    }
    for path in [
        "../private.json",
        "/tmp/private",
        "C:/private",
        "a/../../private",
    ] {
        let mut v = definition("static");
        v["material"] = json!(path);
        assert!(system(&v).is_err());
    }
    let p = PkgReader::parse(
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/atlas-native/rotated0/scene.pkg"),
        )
        .unwrap(),
    )
    .unwrap();
    let tex = p
        .entries()
        .iter()
        .find(|e| e.filename.ends_with(".tex"))
        .unwrap();
    let p = repack(
        &pkg,
        &[("materials/particle.tex", p.read_entry(tex).to_vec())],
    );
    assert!(
        resolve_particle(&p, "particles/probe.json", 1024 * 1024)
            .unwrap_err()
            .contains("static frame zero")
    );
}

#[test]
fn particle_npot_repeat_and_extra_mips_fail_shared_texture_admission() {
    let pkg = fixture("static");
    let base = pkg.read_entry(pkg.find("materials/particle.tex").unwrap());
    let mut tex = base[..83].to_vec();
    for (offset, value) in [
        (22, 0_u32),
        (26, 3),
        (30, 2),
        (34, 3),
        (38, 2),
        (63, 3),
        (67, 2),
        (75, 24),
        (79, 24),
    ] {
        tex[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    tex.extend([255; 24]);
    let p = repack(&pkg, &[("materials/particle.tex", tex.clone())]);
    assert!(
        resolve_particle(&p, "particles/probe.json", 1024)
            .unwrap_err()
            .contains("power-of-two")
    );
    tex[22..26].copy_from_slice(&2_u32.to_le_bytes());
    let p = repack(&pkg, &[("materials/particle.tex", tex)]);
    assert!(resolve_particle(&p, "particles/probe.json", 1024).is_ok());
    let mut tex = base.to_vec();
    tex[59..63].copy_from_slice(&2_u32.to_le_bytes());
    for value in [4_u32, 2, 0, 32, 32] {
        tex.extend(value.to_le_bytes());
    }
    tex.extend([255; 32]);
    let p = repack(&pkg, &[("materials/particle.tex", tex)]);
    assert!(
        resolve_particle(&p, "particles/probe.json", 1024)
            .unwrap_err()
            .contains("base mip")
    );
}
