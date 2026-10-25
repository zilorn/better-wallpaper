//! Strict admission and stateless sampling for a deliberately small 2D subset.
use serde_json::{Map, Value};

use crate::{BlendMode, PkgReader, SceneNode, TexAtlas, TextureImage};

pub const MAX_PARTICLES: u32 = 100_000;
pub const DEFAULT_PARTICLE_LIMIT: u32 = 10_000;
pub const MAX_PARTICLE_SYSTEMS: usize = 64;
pub const MAX_PARTICLE_TEXTURE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_SCENE_PARTICLE_TEXTURE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct ParticleSystem {
    max_count: u32,
    rate: f64,
    burst: u32,
    duration: f64,
    lifetime: f64,
    position: [[f32; 3]; 2],
    velocity: [[f32; 3]; 2],
    size: [f32; 2],
    color: [[f32; 3]; 2],
    occupancy: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleSample {
    pub id: u64,
    pub position: [f32; 2],
    pub size: f32,
    pub color: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct ParticleAsset {
    pub system: ParticleSystem,
    pub texture_path: String,
    pub texture: TextureImage,
    pub blend_mode: BlendMode,
    pub sampling: crate::atlas_animation::SpriteSampling,
}

type Object = Map<String, Value>;
fn object<'a>(v: &'a Value, context: &str, fields: &[&str]) -> Result<&'a Object, String> {
    let o = v
        .as_object()
        .ok_or_else(|| format!("{context}: expected object"))?;
    if let Some(key) = o.keys().find(|k| !fields.contains(&k.as_str())) {
        return Err(format!("{context}: unsupported field {key}"));
    }
    Ok(o)
}
fn number(o: &Object, key: &str, default: Option<f64>, min: f64, max: f64) -> Result<f64, String> {
    let n = match o.get(key) {
        Some(v) => v
            .as_f64()
            .ok_or_else(|| format!("{key}: expected finite number"))?,
        None => default.ok_or_else(|| format!("{key}: explicit value required"))?,
    };
    if !n.is_finite() || !(min..=max).contains(&n) {
        return Err(format!("{key}: expected {min}..={max}"));
    }
    Ok(n)
}
fn integer(o: &Object, key: &str, default: Option<f64>, max: u32) -> Result<u32, String> {
    let n = number(o, key, default, 0., f64::from(max))?;
    if n.fract() != 0. {
        return Err(format!("{key}: expected integer"));
    }
    Ok(n as u32)
}
fn vector(
    o: &Object,
    key: &str,
    default: Option<[f32; 3]>,
    bound: f32,
) -> Result<[f32; 3], String> {
    let Some(v) = o.get(key) else {
        return default.ok_or_else(|| format!("{key}: explicit vector required"));
    };
    let parts = v
        .as_str()
        .ok_or_else(|| format!("{key}: expected three-number string"))?
        .split_whitespace()
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("{key}: invalid vector"))?;
    if parts.len() != 3 || parts.iter().any(|v| !v.is_finite() || v.abs() > bound) {
        return Err(format!("{key}: invalid or over-limit vector"));
    }
    Ok([parts[0], parts[1], parts[2]])
}
fn zero(o: &Object, key: &str) -> Result<(), String> {
    number(o, key, Some(0.), 0., 0.).map(|_| ())
}
fn elements<'a>(o: &'a Object, key: &str, max: usize) -> Result<&'a [Value], String> {
    let a = o
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key}: expected array"))?;
    if a.len() > max {
        return Err(format!("{key}: exceeds {max} elements"));
    }
    Ok(a)
}
fn named<'a>(v: &'a Value, name: &str, fields: &[&str]) -> Result<&'a Object, String> {
    let o = object(v, name, fields)?;
    if o.get("name").and_then(Value::as_str) != Some(name) {
        return Err(format!(
            "unsupported particle element: {}",
            o.get("name").unwrap_or(&Value::Null)
        ));
    }
    Ok(o)
}

/// Called by planning and package-aware compatibility before granting support.
pub fn validate_particle_node(node: &SceneNode) -> Result<(), String> {
    if !node.unknown_fields.is_empty() {
        return Err(format!(
            "unsupported particle node fields: {}",
            node.unknown_fields.join(", ")
        ));
    }
    let t = &node.transform;
    if node.parent.is_some()
        || t.origin_animation.is_some()
        || t.scale_animation.is_some()
        || t.opacity_animation.is_some()
        || node.dynamic_scale.is_some()
    {
        return Err("particle parent/transform timelines are unsupported".into());
    }
    if t.angles.is_some_and(|a| a.x != 0. || a.y != 0.) || t.origin.is_some_and(|o| o.z != 0.) {
        return Err("nonplanar particle transform is unsupported".into());
    }
    if !node.effect_instances.is_empty() || node.texture_override.is_some() {
        return Err("particle effects/texture overrides are unsupported".into());
    }
    Ok(())
}

impl ParticleSystem {
    pub fn parse(bytes: &[u8]) -> Result<(Self, String), String> {
        if bytes.len() > 128 * 1024 {
            return Err("particle JSON exceeds 128 KiB".into());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let root = object(
            &value,
            "particle",
            &[
                "material",
                "maxcount",
                "starttime",
                "flags",
                "animationmode",
                "sequencemultiplier",
                "controlpoint",
                "emitter",
                "initializer",
                "operator",
                "renderer",
            ],
        )?;
        zero(root, "flags")?;
        zero(root, "starttime").map_err(|_| "particle starttime/prewarm is unsupported")?;
        number(root, "sequencemultiplier", Some(1.), 1., 1.)?;
        if root
            .get("animationmode")
            .is_some_and(|v| v.as_str() != Some("sequence"))
        {
            return Err("particle animationmode is unsupported (static frame zero only)".into());
        }
        if let Some(points) = root.get("controlpoint") {
            let points = points
                .as_array()
                .filter(|p| p.len() <= 8)
                .ok_or("invalid controlpoint array")?;
            for p in points {
                let p = object(p, "controlpoint", &["id", "flags", "offset"])?;
                zero(p, "flags")?;
                integer(p, "id", None, 7)?;
                if vector(p, "offset", Some([0.; 3]), 1e6)? != [0.; 3] {
                    return Err("particle control-point offsets are unsupported".into());
                }
            }
        }
        let emitters = elements(root, "emitter", 1)?;
        let e = named(
            emitters.first().ok_or("one boxrandom emitter required")?,
            "boxrandom",
            &[
                "id",
                "name",
                "rate",
                "instantaneous",
                "duration",
                "starttime",
                "distancemin",
                "distancemax",
                "origin",
                "directions",
                "speedmin",
                "speedmax",
                "controlpoint",
                "flags",
            ],
        )?;
        for key in ["speedmin", "speedmax", "controlpoint", "flags", "starttime"] {
            zero(e, key)?;
        }
        if vector(e, "origin", Some([0.; 3]), 1e6)? != [0.; 3]
            || vector(e, "directions", Some([1., 1., 0.]), 1.)? != [1., 1., 0.]
        {
            return Err("particle emitter origin/directions are unsupported".into());
        }
        let position = [
            vector(e, "distancemin", None, 1e6)?,
            vector(e, "distancemax", None, 1e6)?,
        ];
        let rate = number(e, "rate", None, 0., 10_000.)?;
        let burst = integer(e, "instantaneous", Some(0.), MAX_PARTICLES)?;
        if (rate == 0.) == (burst == 0) {
            return Err("choose continuous rate or initial burst".into());
        }
        let mut lifetime = None;
        let mut size = None;
        let mut velocity = None;
        let mut color = None;
        for init in elements(root, "initializer", 4)? {
            let o = object(
                init,
                "initializer",
                &["id", "name", "min", "max", "exponent"],
            )?;
            number(o, "exponent", Some(1.), 1., 1.)?;
            match o.get("name").and_then(Value::as_str) {
                Some("lifetimerandom") if lifetime.is_none() => {
                    let min = number(o, "min", None, 0.001, 3600.)?;
                    number(o, "max", None, min, min)?;
                    lifetime = Some(min);
                }
                Some("sizerandom") if size.is_none() => {
                    let min = number(o, "min", None, 0., 16_384.)?;
                    size = Some([min as f32, number(o, "max", None, min, 16_384.)? as f32]);
                }
                Some("velocityrandom") if velocity.is_none() => {
                    velocity = Some([
                        vector(o, "min", None, 10_000.)?,
                        vector(o, "max", None, 10_000.)?,
                    ]);
                }
                Some("colorrandom") if color.is_none() => {
                    color = Some([vector(o, "min", None, 255.)?, vector(o, "max", None, 255.)?]);
                }
                _ => {
                    return Err(format!(
                        "unsupported or duplicate initializer: {}",
                        o.get("name").unwrap_or(&Value::Null)
                    ));
                }
            }
        }
        let lifetime = lifetime.ok_or("explicit lifetimerandom required")?;
        let velocity = velocity.ok_or("explicit velocityrandom required")?;
        let color = color.ok_or("explicit colorrandom required")?;
        for range in [position, velocity] {
            if range[0][2] != 0. || range[1][2] != 0. || (0..2).any(|a| range[0][a] > range[1][a]) {
                return Err(
                    "particle position/velocity must be ordered XY ranges with Z zero".into(),
                );
            }
        }
        if color.iter().flatten().any(|c| *c < 0.) {
            return Err("particle colors must be in 0..=255".into());
        }
        let ops = elements(root, "operator", 1)?;
        let movement = named(
            ops.first().ok_or("one movement operator required")?,
            "movement",
            &["id", "name", "gravity", "drag", "flags"],
        )?;
        zero(movement, "drag")?;
        zero(movement, "flags")?;
        if vector(movement, "gravity", Some([0.; 3]), 1e6)? != [0.; 3] {
            return Err("particle gravity is unsupported".into());
        }
        let renderers = elements(root, "renderer", 1)?;
        named(
            renderers.first().ok_or("one sprite renderer required")?,
            "sprite",
            &["id", "name"],
        )?;
        let occupancy = if burst > 0 {
            f64::from(burst)
        } else {
            (rate * lifetime).ceil()
        };
        if occupancy > f64::from(MAX_PARTICLES) {
            return Err("particle concurrent birth budget exceeds 100000".into());
        }
        let path = root
            .get("material")
            .and_then(Value::as_str)
            .ok_or("particle material required")?;
        safe_path(path)?;
        Ok((
            Self {
                max_count: integer(root, "maxcount", None, MAX_PARTICLES)?,
                rate,
                burst,
                duration: number(e, "duration", Some(0.), 0., 1e9)?,
                lifetime,
                position,
                velocity,
                size: size.ok_or("explicit sizerandom required")?,
                color,
                occupancy: occupancy as u32,
            },
            path.into(),
        ))
    }

    pub fn capacity(&self) -> u32 {
        self.max_count.min(self.occupancy)
    }

    /// FNV-1a over UTF-8 node ID; stable across reloads, platforms and processes.
    pub fn seed(node_id: &str) -> u64 {
        node_id.bytes().fold(0xcbf29ce484222325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        })
    }

    /// No state or allocation; at most `capacity` callbacks, independent of uptime.
    pub fn sample(
        &self,
        seconds: f64,
        seed: u64,
        capacity: u32,
        mut emit: impl FnMut(ParticleSample),
    ) -> Result<usize, String> {
        if !seconds.is_finite() {
            return Err("particle scene clock must be finite".into());
        }
        let time = seconds;
        let cap = capacity.min(self.capacity());
        if time < 0. || cap == 0 {
            return Ok(0);
        }
        let (first, last, phase) = if self.burst > 0 {
            if time >= self.lifetime {
                return Ok(0);
            }
            (0, u64::from(self.burst - 1), 0.)
        } else {
            let phase = time * self.rate;
            if phase >= (1_u64 << 52) as f64 {
                return Err("particle clock exceeds exact birth identity range".into());
            }
            let end = if self.duration > 0. {
                phase.min(self.duration * self.rate)
            } else {
                phase
            };
            let first = (phase - self.lifetime * self.rate).floor().max(0.) as u64 + 1;
            (first, end.floor() as u64, phase)
        };
        let cycle = u64::from(self.occupancy);
        let mut id = first;
        let mut count = 0;
        while id <= last {
            let residue = if self.burst > 0 { id } else { (id - 1) % cycle };
            if residue >= u64::from(cap) {
                id += cycle - residue;
                continue;
            }
            let age = if self.burst > 0 {
                time
            } else {
                (phase - id as f64) / self.rate
            };
            if age >= 0. && age < self.lifetime {
                let sample =
                    |channel, low: f32, high: f32| low + (high - low) * random(seed, id, channel);
                let position = std::array::from_fn(|a| {
                    sample(a as u64, self.position[0][a], self.position[1][a])
                        + sample(2 + a as u64, self.velocity[0][a], self.velocity[1][a])
                            * age as f32
                });
                let rgb: [f32; 3] = std::array::from_fn(|a| {
                    sample(5 + a as u64, self.color[0][a], self.color[1][a]) / 255.
                });
                emit(ParticleSample {
                    id,
                    position,
                    size: sample(4, self.size[0], self.size[1]),
                    color: [rgb[0], rgb[1], rgb[2], 1.],
                });
                count += 1;
            }
            id += 1;
        }
        Ok(count)
    }
}

fn random(seed: u64, id: u64, channel: u64) -> f32 {
    // SplitMix64 finalizer, with independent counter/channel inputs. Top 24 bits
    // produce a reproducible f32 in [0,1), without RNG state or platform hashes.
    let mut x = seed
        .wrapping_add(id.wrapping_mul(0x9e3779b97f4a7c15))
        .wrapping_add(channel.wrapping_mul(0xd1b54a32d192ed03));
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((x ^ (x >> 31)) >> 40) as f32 / 16_777_216.
}

fn safe_path(path: &str) -> Result<(), String> {
    crate::ir::validate_resource_path(path)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
fn json(package: &PkgReader, path: &str) -> Result<Value, String> {
    safe_path(path)?;
    let entry = package
        .find(path)
        .ok_or_else(|| format!("particle asset missing: {path}"))?;
    let bytes = package.read_entry(entry);
    if bytes.len() > 128 * 1024 {
        return Err(format!("particle JSON exceeds 128 KiB: {path}"));
    }
    serde_json::from_slice(bytes).map_err(|e| format!("{path}: {e}"))
}

pub fn resolve_particle(
    package: &PkgReader,
    path: &str,
    budget: u64,
) -> Result<ParticleAsset, String> {
    safe_path(path)?;
    let entry = package
        .find(path)
        .ok_or_else(|| format!("particle definition missing: {path}"))?;
    let (system, material) = ParticleSystem::parse(package.read_entry(entry))?;
    let value = json(package, &material)?;
    let root = object(&value, "particle material", &["passes"])?;
    let passes = elements(root, "passes", 1)?;
    let pass = object(
        passes.first().ok_or("particle material pass required")?,
        "particle pass",
        &[
            "shader",
            "textures",
            "blending",
            "cullmode",
            "depthtest",
            "depthwrite",
        ],
    )?;
    if pass.get("shader").and_then(Value::as_str) != Some("genericparticle") {
        return Err("particle shader must be genericparticle".into());
    }
    for (key, expected) in [
        ("cullmode", "nocull"),
        ("depthtest", "disabled"),
        ("depthwrite", "disabled"),
    ] {
        if pass.get(key).is_some_and(|v| v.as_str() != Some(expected)) {
            return Err(format!("unsupported particle {key}"));
        }
    }
    let blend_mode = match pass.get("blending").and_then(Value::as_str) {
        Some("translucent") => BlendMode::Translucent,
        Some("additive") => BlendMode::Additive,
        _ => return Err("particle blending must be translucent or additive".into()),
    };
    let textures = elements(pass, "textures", 1)?;
    let texture = textures
        .first()
        .and_then(Value::as_str)
        .ok_or("one particle texture required")?;
    safe_path(texture)?;
    let texture_path = crate::resolve_texture_path(&material, texture);
    let entry = package
        .find(&texture_path)
        .ok_or_else(|| format!("particle texture missing: {texture_path}"))?;
    let atlas = TexAtlas::parse_with_budget(
        package.read_entry(entry),
        budget.min(MAX_PARTICLE_TEXTURE_BYTES),
    )
    .map_err(|e| format!("particle texture: {e}"))?;
    if atlas.images.len() != 1 || !atlas.frames.is_empty() || atlas.is_video {
        return Err(
            "particle texture supports static frame zero only; atlas/video unsupported".into(),
        );
    }
    if atlas.images[0].mipmaps.len() != 1 || atlas.flags & !3 != 0 {
        return Err("particle texture requires one base mip and ordinary point/clamp flags".into());
    }
    if atlas.is_animated
        || atlas.width != atlas.texture_width
        || atlas.height != atlas.texture_height
    {
        return Err("particle texture requires an unpadded static image".into());
    }
    if atlas.flags & 2 == 0 && (!atlas.width.is_power_of_two() || !atlas.height.is_power_of_two()) {
        return Err("particle repeat sampling requires power-of-two texture axes on GLES2".into());
    }
    let mut images = atlas.to_texture_images().map_err(|e| e.to_string())?;
    let texture = images.pop().ok_or("particle texture has no image")?;
    Ok(ParticleAsset {
        system,
        texture_path,
        texture,
        blend_mode,
        sampling: crate::atlas_animation::SpriteSampling {
            nearest: atlas.flags & 1 != 0,
            clamp: atlas.flags & 2 != 0,
        },
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn random_policy_matches_independently_computed_counter_vectors() {
        let seed = super::ParticleSystem::seed("17");
        assert_eq!(seed, 0x07f89107b4ba06f1);
        // Computed with an independent integer implementation of the documented
        // FNV/SplitMix policy; these pin the public reproducibility contract.
        for (channel, upper24) in [
            6584680, 4375024, 8149523, 13261591, 12863048, 2713819, 10220215, 4431598,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                (super::random(seed, 1, channel as u64) * 16_777_216.) as u32,
                upper24
            );
        }
    }
}
