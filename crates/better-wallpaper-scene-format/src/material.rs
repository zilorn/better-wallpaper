use serde_json::Value;

use crate::{SceneParseError, UnsupportedFeature};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Opaque,
    Translucent,
    Additive,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MaterialPass {
    pub blend_mode: BlendMode,
    pub shader: String,
    pub textures: Vec<String>,
    pub unsupported_features: Vec<UnsupportedFeature>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MaterialDefinition {
    pub passes: Vec<MaterialPass>,
    pub unsupported_features: Vec<UnsupportedFeature>,
}

pub fn parse_material_definition(json: &str) -> Result<MaterialDefinition, SceneParseError> {
    let value: Value = serde_json::from_str(json)?;
    let root = value
        .as_object()
        .ok_or_else(|| invalid("material", "expected a JSON object"))?;
    let passes = root
        .get("passes")
        .and_then(Value::as_array)
        .ok_or_else(|| SceneParseError::MissingField("material.passes".into()))?;
    if passes.is_empty() {
        return Err(invalid("material.passes", "expected at least one pass"));
    }
    let passes = passes
        .iter()
        .enumerate()
        .map(|(index, value)| parse_pass(index, value))
        .collect::<Result<Vec<_>, _>>()?;
    let unsupported_features = root
        .keys()
        .filter(|key| key.as_str() != "passes")
        .map(|key| UnsupportedFeature {
            path: format!("material.{key}"),
            feature: "unknown material field".into(),
        })
        .collect();
    Ok(MaterialDefinition {
        passes,
        unsupported_features,
    })
}

fn parse_pass(index: usize, value: &Value) -> Result<MaterialPass, SceneParseError> {
    let field = format!("material.passes[{index}]");
    let pass = value
        .as_object()
        .ok_or_else(|| invalid(&field, "expected a JSON object"))?;
    let blend_mode = match pass
        .get("blending")
        .and_then(Value::as_str)
        .unwrap_or("opaque")
    {
        "opaque" => BlendMode::Opaque,
        "translucent" => BlendMode::Translucent,
        "additive" => BlendMode::Additive,
        other => BlendMode::Unknown(other.to_owned()),
    };
    let shader = pass
        .get("shader")
        .and_then(Value::as_str)
        .ok_or_else(|| SceneParseError::MissingField(format!("{field}.shader")))?;
    if shader.is_empty() || shader.contains('\0') {
        return Err(invalid(
            &format!("{field}.shader"),
            "expected a non-empty shader identifier",
        ));
    }
    let textures = pass.get("textures").map_or(Ok(Vec::new()), |value| {
        let values = value
            .as_array()
            .ok_or_else(|| invalid(&format!("{field}.textures"), "expected an array"))?;
        values
            .iter()
            .enumerate()
            .map(|(texture_index, value)| {
                let texture = value.as_str().ok_or_else(|| {
                    invalid(
                        &format!("{field}.textures[{texture_index}]"),
                        "expected a string",
                    )
                })?;
                validate_texture_reference(&format!("{field}.textures[{texture_index}]"), texture)
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let unsupported_features = pass
        .keys()
        .filter(|key| {
            !matches!(
                key.as_str(),
                "blending"
                    | "combos"
                    | "cullmode"
                    | "depthtest"
                    | "depthwrite"
                    | "shader"
                    | "textures"
            )
        })
        .map(|key| UnsupportedFeature {
            path: format!("{field}.{key}"),
            feature: "unknown material pass field".into(),
        })
        .collect();
    Ok(MaterialPass {
        blend_mode,
        shader: shader.to_owned(),
        textures,
        unsupported_features,
    })
}

fn validate_texture_reference(field: &str, value: &str) -> Result<String, SceneParseError> {
    let normalized = value.replace('\\', "/");
    let has_drive = normalized.len() >= 2
        && normalized.as_bytes()[0].is_ascii_alphabetic()
        && normalized.as_bytes()[1] == b':';
    if normalized.is_empty()
        || normalized.starts_with('/')
        || has_drive
        || normalized.contains('\0')
        || normalized.split('/').any(|part| part == "..")
    {
        return Err(invalid(field, "invalid texture reference"));
    }
    Ok(normalized)
}

fn invalid(field: &str, detail: &str) -> SceneParseError {
    SceneParseError::InvalidValue {
        field: field.into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_observed_image_material() {
        let material = parse_material_definition(r#"{"passes":[{"blending":"translucent","combos":{},"cullmode":"nocull","depthtest":"disabled","depthwrite":"disabled","shader":"genericimage4","textures":["subject"]}]}"#).unwrap();
        assert_eq!(material.passes[0].blend_mode, BlendMode::Translucent);
        assert_eq!(material.passes[0].shader, "genericimage4");
        assert_eq!(material.passes[0].textures, ["subject"]);
        assert!(material.passes[0].unsupported_features.is_empty());
    }

    #[test]
    fn preserves_unknown_semantics_and_rejects_unsafe_references() {
        let material = parse_material_definition(r#"{"passes":[{"blending":"multiply","shader":"custom","textures":["a.tex"],"custom":1}],"extra":1}"#).unwrap();
        assert_eq!(
            material.passes[0].blend_mode,
            BlendMode::Unknown("multiply".into())
        );
        assert_eq!(material.unsupported_features[0].path, "material.extra");
        assert_eq!(
            material.passes[0].unsupported_features[0].path,
            "material.passes[0].custom"
        );
        assert!(
            parse_material_definition(r#"{"passes":[{"shader":"x","textures":["../secret"]}]}"#)
                .is_err()
        );
    }
}
