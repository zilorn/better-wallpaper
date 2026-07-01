use sha1::{Digest, Sha1};

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use crate::{
    SceneParseError,
    ir::{SceneGraph, SceneNodeKind},
    model::{ModelDefinition, parse_model_definition},
    pkg::PkgReader,
};

/// Stable classification used by the runtime when deciding how an entry may be loaded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Scene,
    Model,
    Material,
    Texture,
    Image,
    Video,
    Audio,
    Shader,
    Particle,
    Script,
    Unknown { extension: String },
}

/// A validated package resource. Paths originate from `PkgReader` and are already normalized.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceDescriptor {
    pub path: String,
    pub kind: ResourceKind,
    pub size: u64,
    /// Lowercase SHA-1. This is a cache identity, not a security signature.
    pub content_hash: String,
}

/// Deterministically ordered inventory of all resources in a scene package.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceManifest {
    pub resources: Vec<ResourceDescriptor>,
}

/// A package path used by a scene node and the JSON location that introduced it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ResourceReference {
    pub path: String,
    pub source: String,
}

/// Deterministic result of checking direct scene references against the package index.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceValidationReport {
    pub references: Vec<ResourceReference>,
    pub missing: Vec<ResourceReference>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelResource {
    pub path: String,
    pub definition: ModelDefinition,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelManifest {
    pub models: Vec<ModelResource>,
}

/// Hard upper bound for a CPU-side cache, independent of user quality settings.
pub const MAX_RESOURCE_CACHE_BYTES: u64 = 512 * 1024 * 1024;

/// Stable cache identity. The path prevents unrelated logical resources with equal content
/// from accidentally sharing mutable runtime metadata, while the hash invalidates hot reloads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceCacheKey {
    pub path: String,
    pub content_hash: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResourceCacheError {
    #[error("Resource cache budget must be between 1 and {max} bytes, got {requested}")]
    InvalidBudget { requested: u64, max: u64 },
    #[error("Resource {path} is {size} bytes and exceeds the cache budget of {budget} bytes")]
    ResourceTooLarge {
        path: String,
        size: u64,
        budget: u64,
    },
}

#[derive(Debug)]
struct CachedResource {
    data: Arc<[u8]>,
    last_used: u64,
}

/// CPU-side byte cache with an enforced memory budget and deterministic LRU eviction.
#[derive(Debug)]
pub struct ResourceCache {
    budget: u64,
    used: u64,
    clock: u64,
    entries: HashMap<ResourceCacheKey, CachedResource>,
}

impl ResourceCache {
    pub fn new(budget: u64) -> Result<Self, ResourceCacheError> {
        if budget == 0 || budget > MAX_RESOURCE_CACHE_BYTES {
            return Err(ResourceCacheError::InvalidBudget {
                requested: budget,
                max: MAX_RESOURCE_CACHE_BYTES,
            });
        }
        Ok(Self {
            budget,
            used: 0,
            clock: 0,
            entries: HashMap::new(),
        })
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&mut self, key: &ResourceCacheKey) -> Option<Arc<[u8]>> {
        self.clock = self.clock.saturating_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.last_used = self.clock;
        Some(Arc::clone(&entry.data))
    }

    pub fn insert(
        &mut self,
        key: ResourceCacheKey,
        data: Arc<[u8]>,
    ) -> Result<(), ResourceCacheError> {
        let size = data.len() as u64;
        if size > self.budget {
            return Err(ResourceCacheError::ResourceTooLarge {
                path: key.path,
                size,
                budget: self.budget,
            });
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.used -= previous.data.len() as u64;
        }
        while self.used.saturating_add(size) > self.budget {
            let Some(eviction_key) = self
                .entries
                .iter()
                .min_by(|(left_key, left), (right_key, right)| {
                    left.last_used
                        .cmp(&right.last_used)
                        .then_with(|| left_key.path.cmp(&right_key.path))
                        .then_with(|| left_key.content_hash.cmp(&right_key.content_hash))
                })
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&eviction_key) {
                self.used -= evicted.data.len() as u64;
                tracing::debug!(
                    resource = %eviction_key.path,
                    bytes = evicted.data.len(),
                    "Evicted scene resource from CPU cache"
                );
            }
        }

        self.clock = self.clock.saturating_add(1);
        self.used += size;
        self.entries.insert(
            key,
            CachedResource {
                data,
                last_used: self.clock,
            },
        );
        Ok(())
    }
}

impl ResourceManifest {
    pub fn from_package(package: &PkgReader) -> Self {
        let mut resources = package
            .entries()
            .iter()
            .map(|entry| {
                let bytes = package.read_entry(entry);
                ResourceDescriptor {
                    path: entry.filename.clone(),
                    kind: classify_resource(&entry.filename, entry.ext(), bytes),
                    size: entry.size,
                    content_hash: format!("{:x}", Sha1::digest(bytes)),
                }
            })
            .collect::<Vec<_>>();
        resources.sort_by(|left, right| left.path.cmp(&right.path));
        Self { resources }
    }

    pub fn find(&self, path: &str) -> Option<&ResourceDescriptor> {
        let normalized = path.replace('\\', "/");
        self.resources
            .binary_search_by(|resource| resource.path.as_str().cmp(&normalized))
            .ok()
            .map(|index| &self.resources[index])
    }

    pub fn validate_scene_graph(&self, graph: &SceneGraph) -> ResourceValidationReport {
        self.validate_scene_graph_with_models(graph, None)
    }

    pub fn validate_scene_graph_with_models(
        &self,
        graph: &SceneGraph,
        models: Option<&ModelManifest>,
    ) -> ResourceValidationReport {
        let mut references = BTreeSet::new();
        for (index, node) in graph.nodes.iter().enumerate() {
            let source = format!("objects[{index}]");
            let resource = match &node.kind {
                SceneNodeKind::Image(path)
                | SceneNodeKind::Sound(path)
                | SceneNodeKind::Particle(path)
                    if !path.is_empty() =>
                {
                    Some(path)
                }
                _ => None,
            };
            if let Some(path) = resource {
                references.insert(ResourceReference {
                    path: path.clone(),
                    source: source.clone(),
                });
            }
            for (effect_index, path) in node.effects.iter().enumerate() {
                references.insert(ResourceReference {
                    path: path.clone(),
                    source: format!("{source}.effects[{effect_index}]"),
                });
            }
        }
        if let Some(models) = models {
            for model in &models.models {
                references.insert(ResourceReference {
                    path: model.definition.material.clone(),
                    source: format!("{}.material", model.path),
                });
                if let Some(puppet) = &model.definition.puppet {
                    references.insert(ResourceReference {
                        path: puppet.clone(),
                        source: format!("{}.puppet", model.path),
                    });
                }
            }
        }
        let references = references.into_iter().collect::<Vec<_>>();
        let missing = references
            .iter()
            .filter(|reference| self.find(&reference.path).is_none())
            .cloned()
            .collect();
        ResourceValidationReport {
            references,
            missing,
        }
    }
}

impl ModelManifest {
    pub fn from_package(package: &PkgReader) -> Result<Self, SceneParseError> {
        let mut models = package
            .entries()
            .iter()
            .filter(|entry| entry.filename.starts_with("models/") && entry.ext() == "json")
            .map(|entry| {
                let json = package.read_entry_string(entry).map_err(|error| {
                    SceneParseError::InvalidValue {
                        field: entry.filename.clone(),
                        detail: error.to_string(),
                    }
                })?;
                let definition = parse_model_definition(&json).map_err(|error| {
                    SceneParseError::InvalidValue {
                        field: entry.filename.clone(),
                        detail: error.to_string(),
                    }
                })?;
                Ok(ModelResource {
                    path: entry.filename.clone(),
                    definition,
                })
            })
            .collect::<Result<Vec<_>, SceneParseError>>()?;
        models.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self { models })
    }
}

fn classify_resource(path: &str, extension: &str, bytes: &[u8]) -> ResourceKind {
    let lower_path = path.to_ascii_lowercase();
    if lower_path == "scene.json" {
        return ResourceKind::Scene;
    }
    if lower_path.contains("particle") && extension == "json" {
        return ResourceKind::Particle;
    }
    if lower_path.starts_with("models/") && extension == "json" {
        return ResourceKind::Model;
    }
    if lower_path.starts_with("materials/") && extension == "json" {
        return ResourceKind::Material;
    }

    match extension {
        "tex" | "dds" => ResourceKind::Texture,
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tga" => ResourceKind::Image,
        "mp4" | "webm" | "mkv" | "avi" | "mov" => ResourceKind::Video,
        "flac" | "wav" | "ogg" | "mp3" | "m4a" | "aac" => ResourceKind::Audio,
        "frag" | "vert" | "glsl" | "hlsl" => ResourceKind::Shader,
        "js" => ResourceKind::Script,
        "mdl" => ResourceKind::Model,
        "json" if bytes.starts_with(b"{") || bytes.starts_with(b"[") => ResourceKind::Unknown {
            extension: extension.to_owned(),
        },
        _ => ResourceKind::Unknown {
            extension: extension.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sized_string(value: &str) -> Vec<u8> {
        let mut result = (value.len() as u32).to_le_bytes().to_vec();
        result.extend_from_slice(value.as_bytes());
        result
    }

    fn package(entries: &[(&str, &[u8])]) -> PkgReader {
        let mut bytes = sized_string("PKGV0001");
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        let mut offset = 0u32;
        for (name, data) in entries {
            bytes.extend_from_slice(&sized_string(name));
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            offset += data.len() as u32;
        }
        for (_, data) in entries {
            bytes.extend_from_slice(data);
        }
        PkgReader::parse(bytes).unwrap()
    }

    #[test]
    fn manifest_is_sorted_classified_and_hashed() {
        let package = package(&[
            ("textures\\background.tex", b"TEXV"),
            ("scene.json", b"{}"),
            ("effects/custom.frag", b"shader"),
            ("data/blob.bin", b"opaque"),
        ]);
        let manifest = ResourceManifest::from_package(&package);

        assert_eq!(manifest.resources[0].path, "data/blob.bin");
        assert_eq!(manifest.resources[1].kind, ResourceKind::Shader);
        assert_eq!(
            manifest.find("scene.json").unwrap().kind,
            ResourceKind::Scene
        );
        assert_eq!(
            manifest.find("textures\\background.tex").unwrap().kind,
            ResourceKind::Texture
        );
        assert_eq!(manifest.resources[0].content_hash.len(), 40);
        assert!(matches!(
            manifest.resources[0].kind,
            ResourceKind::Unknown { ref extension } if extension == "bin"
        ));
    }

    #[test]
    fn reports_missing_scene_resources_deterministically() {
        let package = package(&[("scene.json", b"{}"), ("models/bg.json", b"{}")]);
        let manifest = ResourceManifest::from_package(&package);
        let graph = crate::ir::parse_scene_graph(
            r#"{"objects":[
                {"image":"models\\bg.json","effects":[{"file":"effects/missing.json"}]},
                {"sound":"audio/missing.ogg"}
            ]}"#,
        )
        .unwrap();
        let report = manifest.validate_scene_graph(&graph);

        assert_eq!(report.references.len(), 3);
        assert_eq!(report.missing.len(), 2);
        assert_eq!(report.missing[0].path, "audio/missing.ogg");
        assert_eq!(report.missing[1].path, "effects/missing.json");
    }

    #[test]
    fn parses_models_and_validates_transitive_resources() {
        let package = package(&[
            ("scene.json", b"{}"),
            (
                "models/bg.json",
                br#"{"autosize":true,"cropoffset":"1 2","material":"materials/bg.json","puppet":"models/bg.mdl"}"#,
            ),
            ("materials/bg.json", b"{}"),
        ]);
        let manifest = ResourceManifest::from_package(&package);
        let models = ModelManifest::from_package(&package).unwrap();
        let graph =
            crate::ir::parse_scene_graph(r#"{"objects":[{"image":"models/bg.json"}]}"#).unwrap();
        let report = manifest.validate_scene_graph_with_models(&graph, Some(&models));

        assert_eq!(models.models.len(), 1);
        assert_eq!(models.models[0].definition.crop_offset.unwrap().x, 1.0);
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].path, "models/bg.mdl");
        assert_eq!(report.missing[0].source, "models/bg.json.puppet");
    }

    fn cache_key(path: &str) -> ResourceCacheKey {
        ResourceCacheKey {
            path: path.to_owned(),
            content_hash: format!("hash-{path}"),
        }
    }

    #[test]
    fn cache_enforces_budget_and_evicts_least_recently_used() {
        let mut cache = ResourceCache::new(6).unwrap();
        let first = cache_key("first");
        let second = cache_key("second");
        let third = cache_key("third");
        cache.insert(first.clone(), Arc::from([1_u8; 3])).unwrap();
        cache.insert(second.clone(), Arc::from([2_u8; 3])).unwrap();
        assert!(cache.get(&first).is_some());

        cache.insert(third.clone(), Arc::from([3_u8; 3])).unwrap();
        assert!(cache.get(&second).is_none());
        assert!(cache.get(&first).is_some());
        assert!(cache.get(&third).is_some());
        assert_eq!(cache.used(), 6);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn cache_replacement_updates_accounting_and_rejects_unsafe_budgets() {
        assert!(matches!(
            ResourceCache::new(0),
            Err(ResourceCacheError::InvalidBudget { .. })
        ));
        assert!(ResourceCache::new(MAX_RESOURCE_CACHE_BYTES + 1).is_err());

        let mut cache = ResourceCache::new(4).unwrap();
        let key = cache_key("same");
        cache.insert(key.clone(), Arc::from([1_u8; 3])).unwrap();
        cache.insert(key, Arc::from([2_u8; 2])).unwrap();
        assert_eq!(cache.used(), 2);
        assert_eq!(cache.len(), 1);

        let error = cache
            .insert(cache_key("large"), Arc::from([0_u8; 5]))
            .unwrap_err();
        assert!(matches!(error, ResourceCacheError::ResourceTooLarge { .. }));
        assert_eq!(cache.used(), 2);
    }
}
