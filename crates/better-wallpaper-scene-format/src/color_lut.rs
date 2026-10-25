//! Bounded admission of the shipped `ccsimple` LUT-only post-processing pass.
use std::sync::Arc;

use serde_json::Value;

use crate::{PkgReader, TexVolume, resolve_texture_path};

pub const MAX_COLOR_LUT_BYTES: u64 = 8 * 1024 * 1024;

// Owned adapter used to exercise the fullscreen fragment stage on image layers.
// Other package-supplied vertex programs cannot inherit this shader's admission.
const LUT_VERTEX_ADAPTER: &[u8] = b"attribute vec3 a_Position;\nuniform mat4 g_ModelViewProjectionMatrix;\nattribute vec2 a_TexCoord;\nvarying vec2 v_TexCoord;\nvoid main() { gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix); v_TexCoord = a_TexCoord; }\n";

#[derive(Debug, Clone)]
pub struct ColorLut {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    /// RGBA8, x fastest, followed by y and z. No transfer-function conversion.
    pub pixels: Arc<[u8]>,
    pub nearest: bool,
    pub clamp: bool,
}

impl ColorLut {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let volume = TexVolume::parse_with_budget(bytes, MAX_COLOR_LUT_BYTES)
            .map_err(|error| error.to_string())?;
        if volume.flags & !0x43 != 0 {
            return Err(
                "Unsupported color LUT flags; only point/clamp volume flags are admitted".into(),
            );
        }
        if volume.mipmaps.len() != 1 {
            return Err("Color LUT requires exactly one base mip".into());
        }
        let mip = &volume.mipmaps[0];
        if [mip.width, mip.height, mip.depth]
            .iter()
            .any(|axis| !(2..=128).contains(axis))
        {
            return Err("Color LUT axes must be in 2..=128".into());
        }
        Ok(Self {
            width: mip.width,
            height: mip.height,
            depth: mip.depth,
            pixels: mip.data.clone(),
            nearest: volume.flags & 1 != 0,
            clamp: volume.flags & 2 != 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColorLutBinding {
    pub effect: String,
    pub path: String,
    pub amount: f32,
}

fn json(package: &PkgReader, path: &str) -> Option<Value> {
    let entry = package.find(path)?;
    let bytes = package.read_entry(entry);
    (bytes.len() <= 2 * 1024 * 1024)
        .then(|| serde_json::from_slice(bytes).ok())
        .flatten()
}

/// A filename alone never grants support. Resolve a single effect/material pass
/// and admit only the known LUT shader with static bounded bindings. Other
/// effects retain their existing unsupported diagnostics.
pub fn resolve_color_lut(
    package: &PkgReader,
    effects: &[Value],
) -> Result<Option<ColorLutBinding>, String> {
    if effects.len() > 64 {
        return Err("Color LUT inspection is limited to 64 effect instances per node".into());
    }
    let visible: Vec<_> = effects
        .iter()
        .filter(|effect| {
            let v = &effect["visible"];
            v.as_bool().or_else(|| v["value"].as_bool()) != Some(false)
        })
        .collect();
    for effect in &visible {
        let Some(file) = effect["file"].as_str() else {
            continue;
        };
        let Some(definition) = json(package, file) else {
            continue;
        };
        let Some(passes) = definition["passes"].as_array() else {
            continue;
        };
        if passes.len() > 32 {
            return Err("Color LUT inspection is limited to 32 effect passes".into());
        }
        let materials: Vec<_> = passes
            .iter()
            .filter_map(|pass| pass["material"].as_str())
            .filter_map(|path| json(package, path))
            .collect();
        if !materials.iter().any(|material| {
            material["passes"]
                .as_array()
                .is_some_and(|passes| passes.iter().any(|pass| pass["shader"] == "ccsimple"))
        }) {
            continue;
        }
        if visible.len() != 1 || passes.len() != 1 || materials.len() != 1 {
            return Err(
                "Color LUT requires one visible effect and one effect/material pass".into(),
            );
        }
        if definition.get("fbos").is_some()
            || passes[0]
                .as_object()
                .is_none_or(|pass| pass.keys().any(|key| key != "material"))
            || package.find("shaders/ccsimple.frag").is_some()
        {
            return Err("Color LUT render targets, pass overrides and custom fragment shaders are unsupported".into());
        }
        if let Some(vertex) = package.find("shaders/ccsimple.vert")
            && package.read_entry(vertex) != LUT_VERTEX_ADAPTER
        {
            return Err("Custom color LUT vertex shaders are unsupported".into());
        }
        for (values, allowed) in [
            (
                &definition,
                &[
                    "version",
                    "replacementkey",
                    "name",
                    "description",
                    "group",
                    "preview",
                    "passes",
                    "dependencies",
                    "gizmos",
                ][..],
            ),
            (&materials[0], &["passes"][..]),
            (*effect, &["file", "name", "id", "visible", "passes"][..]),
        ] {
            let object = values
                .as_object()
                .ok_or("Color LUT definition must be an object")?;
            if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
                return Err(format!("Unsupported color LUT definition field {key}"));
            }
        }
        let material_passes = materials[0]["passes"]
            .as_array()
            .ok_or("Color LUT material is missing passes")?;
        if material_passes.len() != 1 {
            return Err("Color LUT requires one material pass".into());
        }
        let pass = &material_passes[0];
        let overrides = effect["passes"]
            .as_array()
            .ok_or("Color LUT instance is missing passes")?;
        if overrides.len() != 1 {
            return Err("Color LUT requires one instance pass".into());
        }
        let instance = &overrides[0];
        for (values, allowed) in [
            (
                pass,
                &[
                    "shader",
                    "blending",
                    "depthtest",
                    "depthwrite",
                    "cullmode",
                    "textures",
                    "combos",
                    "constantshadervalues",
                ][..],
            ),
            (
                instance,
                &["textures", "combos", "constantshadervalues"][..],
            ),
        ] {
            let object = values
                .as_object()
                .ok_or("Color LUT pass must be an object")?;
            if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
                return Err(format!("Unsupported color LUT pass field {key}"));
            }
        }
        if pass.get("blending").is_some_and(|value| value != "opaque") {
            return Err("Color LUT pass requires opaque intermediate blending".into());
        }
        let mut lut = false;
        let mut amount = 1.0;
        let mut texture = pass["textures"][1].as_str();
        for values in [pass, instance] {
            if let Some(combos) = values.get("combos") {
                for (name, value) in combos.as_object().ok_or("Invalid LUT combos")? {
                    match (name.to_ascii_uppercase().as_str(), value.as_u64()) {
                        ("LUT", Some(0)) => lut = false,
                        ("LUT", Some(1)) => lut = true,
                        ("COL" | "HDR", Some(0)) => {}
                        _ => return Err(format!("Unsupported color LUT combo {name}: {value}")),
                    }
                }
            }
            if let Some(constants) = values.get("constantshadervalues") {
                for (name, value) in constants.as_object().ok_or("Invalid LUT constants")? {
                    if name != "lutparams" {
                        return Err(format!("Unsupported color LUT constant {name}"));
                    }
                    amount = value
                        .as_f64()
                        .ok_or("Color LUT amount must be a static number")?;
                    if !(0.0..=1.0).contains(&amount) {
                        return Err("Color LUT amount must be in 0..=1".into());
                    }
                }
            }
        }
        if !lut {
            return Err("Color LUT requires explicit LUT: 1".into());
        }
        if let Some(textures) = instance.get("textures") {
            let textures = textures.as_array().ok_or("Invalid LUT textures")?;
            if textures.len() > 2 || textures.first().is_some_and(|v| !v.is_null()) {
                return Err("Color LUT texture zero must retain the effect input".into());
            }
            if let Some(value) = textures.get(1).filter(|v| !v.is_null()) {
                texture = Some(value.as_str().ok_or("Invalid LUT texture reference")?);
            }
        }
        if pass["textures"]
            .as_array()
            .is_some_and(|textures| textures.len() > 2)
        {
            return Err("Color LUT material has extra textures".into());
        }
        let texture = texture.ok_or("Color LUT texture one is missing")?;
        // Use the same path validation as material references before resolution.
        let safe = crate::material::validate_texture_reference("color LUT texture", texture)
            .map_err(|error| error.to_string())?;
        if safe.starts_with('_') {
            return Err("Color LUT requires a package texture".into());
        }
        let path = resolve_texture_path("", &safe);
        return Ok(Some(ColorLutBinding {
            effect: file.into(),
            path,
            amount: amount as f32,
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn pkg(mut entries: BTreeMap<String, Vec<u8>>) -> PkgReader {
        let mut index = Vec::new();
        let mut payload = Vec::new();
        let string = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        };
        let mut bytes = Vec::new();
        string(&mut bytes, "PKGV0001");
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (path, data) in &mut entries {
            string(&mut index, path);
            index.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            index.extend_from_slice(&(data.len() as u32).to_le_bytes());
            payload.append(data);
        }
        bytes.extend(index);
        bytes.extend(payload);
        PkgReader::parse(bytes).unwrap()
    }

    fn fixture() -> (BTreeMap<String, Vec<u8>>, Vec<Value>) {
        let package = PkgReader::parse(
            include_bytes!("../../../tests/fixtures/lut-native/mixed/scene.pkg").to_vec(),
        )
        .unwrap();
        let entries = package
            .entries()
            .iter()
            .map(|e| (e.filename.clone(), package.read_entry(e).to_vec()))
            .collect();
        let scene: Value = serde_json::from_str(
            &package
                .read_entry_string(package.find("scene.json").unwrap())
                .unwrap(),
        )
        .unwrap();
        (
            entries,
            scene["objects"][0]["effects"].as_array().unwrap().clone(),
        )
    }

    #[test]
    fn lut_volume_admission_preserves_voxels_and_rejects_mips_and_flags() {
        let (entries, _) = fixture();
        let data = &entries["materials/probe-lut.tex"];
        let lut = ColorLut::parse(data).unwrap();
        assert_eq!((lut.width, lut.height, lut.depth), (32, 32, 32));
        assert!(lut.clamp);
        assert!(!lut.nearest);
        assert_eq!(lut.pixels.len(), 32 * 32 * 32 * 4);
        let mut bad = data.clone();
        bad[22..26].copy_from_slice(&0x4au32.to_le_bytes());
        assert!(ColorLut::parse(&bad).unwrap_err().contains("flags"));
        assert!(
            ColorLut::parse(include_bytes!(
                "../../../tests/fixtures/volume-textures/noncubic-raw.tex"
            ))
            .unwrap_err()
            .contains("one base mip")
        );
    }

    #[test]
    fn invalid_lut_payload_does_not_grant_l3_metadata() {
        let (mut entries, _) = fixture();
        let scene = String::from_utf8(entries["scene.json"].clone()).unwrap();
        entries.insert("materials/probe-lut.tex".into(), b"broken volume".to_vec());
        let package = pkg(entries);
        let metadata = crate::analyse_scene_with_package(&scene, &package).unwrap();
        assert!(metadata.supported_effects.is_empty());
        assert_eq!(
            metadata.unsupported_effects,
            ["effects/probe-lut/effect.json"]
        );
        assert!(metadata.unsupported_textures[0].contains("probe-lut.tex"));
    }

    #[test]
    fn lut_binding_resolves_material_defaults_and_instance_overrides() {
        let (entries, mut effects) = fixture();
        let package = pkg(entries);
        let binding = resolve_color_lut(&package, &effects).unwrap().unwrap();
        assert_eq!(binding.path, "materials/probe-lut.tex");
        assert_eq!(binding.amount, 0.375);
        effects[0]["passes"][0]["constantshadervalues"]["lutparams"] = json!(0);
        assert_eq!(
            resolve_color_lut(&package, &effects)
                .unwrap()
                .unwrap()
                .amount,
            0.0
        );
        effects[0]["visible"] = json!({"value":false});
        assert!(resolve_color_lut(&package, &effects).unwrap().is_none());
    }

    #[test]
    fn lut_binding_rejects_unsupported_parameters_and_unsafe_paths() {
        let (entries, effects) = fixture();
        let package = pkg(entries);
        for (field, value) in [
            ("lutparams", json!(-0.1)),
            ("lutparams", json!(1.1)),
            ("lutparams", json!({"value":0.5,"script":"ignored"})),
            ("params", json!("1 1 1 0")),
        ] {
            let mut invalid = effects.clone();
            invalid[0]["passes"][0]["constantshadervalues"] = json!({field:value});
            assert!(resolve_color_lut(&package, &invalid).is_err());
        }
        for path in [
            "../escape",
            "/etc/passwd",
            "C:/secret",
            "_rt_FullFrameBuffer",
            "x/../../escape",
        ] {
            let mut invalid = effects.clone();
            invalid[0]["passes"][0]["textures"][1] = json!(path);
            assert!(resolve_color_lut(&package, &invalid).is_err(), "{path}");
        }
        for combos in [
            json!({"LUT":0}),
            json!({"HDR":1}),
            json!({"COL":1}),
            json!({"LUT":true}),
            json!({"OTHER":0}),
        ] {
            let mut invalid = effects.clone();
            invalid[0]["passes"][0]["combos"] = combos;
            assert!(resolve_color_lut(&package, &invalid).is_err());
        }
        let mut invalid = effects.clone();
        invalid.push(json!({"file":"effects/shine/effect.json"}));
        assert!(resolve_color_lut(&package, &invalid).is_err());
    }

    #[test]
    fn lut_binding_does_not_trust_names_or_custom_shader_code() {
        let (mut entries, effects) = fixture();
        entries.insert("shaders/ccsimple.frag".into(), b"custom shader".to_vec());
        assert!(resolve_color_lut(&pkg(entries.clone()), &effects).is_err());
        entries.remove("shaders/ccsimple.frag");
        entries.insert(
            "materials/effects/probe-lut.json".into(),
            serde_json::to_vec(&json!({"passes":[{"shader":"custom"}]})).unwrap(),
        );
        assert!(
            resolve_color_lut(&pkg(entries), &effects)
                .unwrap()
                .is_none()
        );
    }
}
