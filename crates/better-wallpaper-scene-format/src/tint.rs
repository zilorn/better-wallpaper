//! Admission of the measured, single-pass LDR Tint operation.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::{PkgReader, ScalarAnimation, TexTexture, TextureImage, resolve_texture_path};

#[derive(Debug, Clone)]
pub struct TintEffect {
    pub color: [f32; 3],
    pub amount: f32,
    pub amount_animation: Option<ScalarAnimation>,
    /// Native BLENDMODE: normal (0), multiply (2), screen (7), tint (30).
    pub blend_mode: u32,
    pub mask: Option<TintMask>,
}

#[derive(Debug, Clone)]
pub struct TintMask {
    pub path: String,
    pub image: TextureImage,
    pub uv_scale: [f32; 2],
    pub nearest: bool,
}

#[derive(Default)]
pub struct TintMaskCache {
    masks: BTreeMap<String, TintMask>,
    bytes: u64,
}

fn fields(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("Tint expects an object")?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unsupported Tint field: {key}"));
    }
    Ok(())
}

fn json(package: &PkgReader, path: &str) -> Result<Value, String> {
    let entry = package
        .find(path)
        .ok_or_else(|| format!("Missing Tint definition: {path}"))?;
    let bytes = package.read_entry(entry);
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Tint JSON exceeds 2 MiB".into());
    }
    serde_json::from_slice(bytes).map_err(|error| format!("Tint {path}: {error}"))
}

/// Unsupported instances preserve the base layer and receive a concrete report.
/// No filename-only support or arbitrary shader/pass reordering is granted.
pub fn resolve_tint(
    package: &PkgReader,
    effects: &[Value],
    cache: &mut TintMaskCache,
) -> Result<Option<TintEffect>, String> {
    let active = |effect: &&Value| {
        crate::ir::unwrap_script_value(&effect["visible"]).as_bool() != Some(false)
    };
    let Some(effect) = effects
        .iter()
        .filter(active)
        .find(|e| e["file"] == "effects/tint/effect.json")
    else {
        return Ok(None);
    };
    if effects.len() > 64 || effects.iter().filter(active).count() != 1 {
        return Err("Tint requires one visible effect; effect stacks are unsupported".into());
    }
    fields(effect, &["file", "id", "name", "visible", "passes"])?;
    if effect["visible"].get("script").is_some() {
        return Err("Script-bound Tint visibility is unsupported".into());
    }
    let definition = json(package, "effects/tint/effect.json")?;
    fields(
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
        ],
    )?;
    let passes = definition["passes"]
        .as_array()
        .ok_or("Tint is missing effect passes")?;
    if passes.len() != 1 {
        return Err("Tint requires one effect pass".into());
    }
    fields(&passes[0], &["material"])?;
    let path = passes[0]["material"]
        .as_str()
        .ok_or("Tint material is missing")?;
    crate::material::validate_texture_reference("Tint material", path)
        .map_err(|e| e.to_string())?;
    let material = json(package, path)?;
    fields(&material, &["passes"])?;
    let passes = material["passes"]
        .as_array()
        .ok_or("Tint is missing material passes")?;
    if passes.len() != 1 {
        return Err("Tint requires one material pass".into());
    }
    let pass = &passes[0];
    fields(
        pass,
        &[
            "shader",
            "blending",
            "depthtest",
            "depthwrite",
            "cullmode",
            "combos",
            "constantshadervalues",
            "textures",
        ],
    )?;
    if pass["shader"] != "effects/tint"
        || pass.get("blending").is_some_and(|v| v != "normal")
        || pass.get("depthtest").is_some_and(|v| v != "disabled")
        || pass.get("depthwrite").is_some_and(|v| v != "disabled")
        || pass.get("cullmode").is_some_and(|v| v != "nocull")
    {
        return Err("Unsupported Tint shader or render state".into());
    }
    for (name, expected) in [
        (
            "shaders/effects/tint.frag",
            "02b9397cec32de6f0d8caa2e1af7c07510752d474bd64048c9d78f55926973f1",
        ),
        (
            "shaders/effects/tint.vert",
            "62b2f5853fc565d706c1b6c789981385254f8195e0e1579c3d737796b41ddf2a",
        ),
        (
            "shaders/common_blending.h",
            "f1cf5473991614784a7feafbd34c65b83a27851bfe9d763ca9c8c3fcbb2dcc6b",
        ),
    ] {
        if let Some(entry) = package.find(name)
            && format!("{:x}", Sha256::digest(package.read_entry(entry))) != expected
        {
            return Err(format!("Unverified Tint shader override: {name}"));
        }
    }
    let overrides = effect["passes"]
        .as_array()
        .ok_or("Tint instance is missing passes")?;
    if overrides.len() != 1 {
        return Err("Tint requires one instance pass".into());
    }
    fields(
        &overrides[0],
        &["combos", "constantshadervalues", "textures"],
    )?;
    let mut result = TintEffect {
        color: [1., 0., 0.],
        amount: 1.,
        amount_animation: None,
        blend_mode: 30,
        mask: None,
    };
    let mut mask_enabled = false;
    let mut texture = None;
    for source in [pass, &overrides[0]] {
        if let Some(combos) = source.get("combos") {
            for (key, value) in combos.as_object().ok_or("Invalid Tint combos")? {
                match (key.as_str(), value.as_u64()) {
                    ("BLENDMODE", Some(mode @ (0 | 2 | 7 | 30))) => result.blend_mode = mode as u32,
                    ("MASK", Some(enabled @ (0 | 1))) => mask_enabled = enabled == 1,
                    _ => return Err(format!("Unsupported Tint combo: {key}={value}")),
                }
            }
        }
        if let Some(constants) = source.get("constantshadervalues") {
            for (key, value) in constants.as_object().ok_or("Invalid Tint constants")? {
                match key.as_str() {
                    "alpha" => {
                        crate::ir::validate_linear_animation(value, "Tint.alpha", 1)
                            .map_err(|e| e.to_string())?;
                        if value["animation"]["relative"] == true {
                            return Err("Relative Tint alpha is unsupported".into());
                        }
                        if value.get("script").is_some() {
                            return Err("Script-bound Tint alpha is unsupported".into());
                        }
                        result.amount = crate::ir::number_value(
                            "Tint.alpha",
                            crate::ir::unwrap_script_value(value),
                        )
                        .map_err(|e| e.to_string())?;
                        result.amount_animation =
                            crate::ir::parse_scalar_animation(value, "Tint.alpha")
                                .map_err(|e| e.to_string())?;
                        if !(0.0..=1.0).contains(&result.amount)
                            || result.amount_animation.as_ref().is_some_and(|a| {
                                a.keyframes.iter().any(|k| !(0.0..=1.0).contains(&k.value))
                            })
                        {
                            return Err("Tint alpha must be in 0..=1".into());
                        }
                    }
                    "color" => {
                        if value.get("script").is_some() || value.get("animation").is_some() {
                            return Err("Animated/script-bound Tint color is unsupported".into());
                        }
                        let color = crate::ir::parse_vec3("Tint.color", value)
                            .map_err(|e| e.to_string())?;
                        result.color = [color.x, color.y, color.z];
                        if result.color.iter().any(|v| !(0.0..=1.0).contains(v)) {
                            return Err("Tint color must be LDR RGB in 0..=1".into());
                        }
                    }
                    _ => return Err(format!("Unsupported Tint constant: {key}")),
                }
            }
        }
        if let Some(textures) = source.get("textures") {
            let textures = textures.as_array().ok_or("Invalid Tint textures")?;
            if textures.len() > 2 || textures.first().is_some_and(|v| !v.is_null()) {
                return Err("Tint texture zero must retain its input".into());
            }
            if let Some(value) = textures.get(1).filter(|v| !v.is_null()) {
                texture = Some(
                    value
                        .as_str()
                        .ok_or("Tint mask must be a static texture path")?,
                );
            }
        }
    }
    if mask_enabled {
        let texture = texture.ok_or("Tint MASK requires texture one")?;
        let safe = crate::material::validate_texture_reference("Tint mask", texture)
            .map_err(|e| e.to_string())?;
        if safe.starts_with('_') {
            return Err("Tint mask must be a package image".into());
        }
        let path = resolve_texture_path("", &safe);
        if let Some(mask) = cache.masks.get(&path) {
            result.mask = Some(mask.clone());
            return Ok(Some(result));
        }
        let entry = package
            .find(&path)
            .ok_or("Tint mask is missing from package")?;
        let tex = TexTexture::parse_with_budget(package.read_entry(entry), 16 * 1024 * 1024)
            .map_err(|e| e.to_string())?;
        if tex.flags != 3
            || tex.format != crate::TexFormat::ARGB8888
            || tex.mipmaps.len() != 1
            || !tex.frames.is_empty()
        {
            return Err("Tint mask requires one static base image with point/clamp sampling; native linear filtering is not yet matched".into());
        }
        let uv_scale = [
            tex.width as f32 / tex.texture_width as f32,
            tex.height as f32 / tex.texture_height as f32,
        ];
        let image = tex.to_texture_image().map_err(|e| e.to_string())?;
        let bytes = image
            .levels
            .iter()
            .map(|mip| mip.data.len() as u64)
            .sum::<u64>();
        if cache.bytes + bytes > 32 * 1024 * 1024 {
            return Err("Scene Tint masks exceed 32 MiB".into());
        }
        let mask = TintMask {
            path: path.clone(),
            image,
            uv_scale,
            nearest: tex.flags & 1 != 0,
        };
        cache.bytes += bytes;
        cache.masks.insert(path, mask.clone());
        result.mask = Some(mask);
    }
    Ok(Some(result))
}
