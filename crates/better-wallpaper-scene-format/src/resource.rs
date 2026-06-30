use sha1::{Digest, Sha1};

use crate::pkg::PkgReader;

/// Stable classification used by the runtime when deciding how an entry may be loaded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Scene,
    Model,
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
}

fn classify_resource(path: &str, extension: &str, bytes: &[u8]) -> ResourceKind {
    let lower_path = path.to_ascii_lowercase();
    if lower_path == "scene.json" {
        return ResourceKind::Scene;
    }
    if lower_path.contains("particle") && extension == "json" {
        return ResourceKind::Particle;
    }

    match extension {
        "tex" | "dds" => ResourceKind::Texture,
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tga" => ResourceKind::Image,
        "mp4" | "webm" | "mkv" | "avi" | "mov" => ResourceKind::Video,
        "flac" | "wav" | "ogg" | "mp3" | "m4a" | "aac" => ResourceKind::Audio,
        "frag" | "vert" | "glsl" | "hlsl" => ResourceKind::Shader,
        "js" => ResourceKind::Script,
        "mdl" => ResourceKind::Model,
        "json" if bytes.starts_with(b"{") || bytes.starts_with(b"[") => ResourceKind::Model,
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
}
