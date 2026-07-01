use serde_json::Value;

use crate::{
    error::SceneParseError,
    ir::{UnsupportedFeature, Vec2},
};

/// Validated, version-independent representation of a `models/*.json` resource.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelDefinition {
    pub autosize: bool,
    pub crop_offset: Option<Vec2>,
    pub material: String,
    pub puppet: Option<String>,
    pub unsupported_features: Vec<UnsupportedFeature>,
}

pub fn parse_model_definition(model_json: &str) -> Result<ModelDefinition, SceneParseError> {
    let root: Value = serde_json::from_str(model_json)?;
    let root = root
        .as_object()
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: "model".into(),
            detail: "expected a JSON object".into(),
        })?;

    let material = resource_path(
        "model.material",
        root.get("material")
            .and_then(Value::as_str)
            .ok_or_else(|| SceneParseError::MissingField("model.material".into()))?,
    )?;
    let puppet = root
        .get("puppet")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| SceneParseError::InvalidValue {
                    field: "model.puppet".into(),
                    detail: "expected a resource path string".into(),
                })
                .and_then(|path| resource_path("model.puppet", path))
        })
        .transpose()?;
    let crop_offset = root
        .get("cropoffset")
        .map(|value| parse_vec2("model.cropoffset", value))
        .transpose()?;

    let unsupported_features = root
        .keys()
        .filter(|key| {
            !matches!(
                key.as_str(),
                "autosize" | "cropoffset" | "material" | "puppet"
            )
        })
        .map(|key| UnsupportedFeature {
            path: format!("model.{key}"),
            feature: "unknown model field".into(),
        })
        .collect();

    Ok(ModelDefinition {
        autosize: root
            .get("autosize")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| SceneParseError::InvalidValue {
                        field: "model.autosize".into(),
                        detail: "expected a boolean".into(),
                    })
            })
            .transpose()?
            .unwrap_or(false),
        crop_offset,
        material,
        puppet,
        unsupported_features,
    })
}

fn parse_vec2(field: &str, value: &Value) -> Result<Vec2, SceneParseError> {
    let values = match value {
        Value::String(value) => value
            .split_whitespace()
            .map(|part| part.parse::<f32>().ok())
            .collect::<Option<Vec<_>>>(),
        Value::Array(values) => values
            .iter()
            .map(|value| value.as_f64().map(|number| number as f32))
            .collect::<Option<Vec<_>>>(),
        _ => None,
    }
    .filter(|values| values.len() == 2)
    .filter(|values| values.iter().all(|value| value.is_finite()))
    .ok_or_else(|| SceneParseError::InvalidValue {
        field: field.into(),
        detail: "expected 2 finite numeric components".into(),
    })?;
    Ok(Vec2 {
        x: values[0],
        y: values[1],
    })
}

fn resource_path(field: &str, path: &str) -> Result<String, SceneParseError> {
    let normalized = path.replace('\\', "/");
    let has_drive = normalized.len() >= 2
        && normalized.as_bytes()[0].is_ascii_alphabetic()
        && normalized.as_bytes()[1] == b':';
    if normalized.is_empty()
        || normalized.starts_with('/')
        || has_drive
        || normalized.contains('\0')
        || normalized.split('/').any(|part| part == "..")
    {
        return Err(SceneParseError::InvalidValue {
            field: field.into(),
            detail: format!("invalid resource path: {path}"),
        });
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_observed_image_model_fields() {
        let model = parse_model_definition(
            r#"{
                "autosize": true,
                "cropoffset": "-37.00000 -84.00000",
                "material": "materials\\subject.json",
                "puppet": "models/subject_puppet.mdl"
            }"#,
        )
        .unwrap();

        assert!(model.autosize);
        assert_eq!(model.crop_offset.unwrap(), Vec2 { x: -37.0, y: -84.0 });
        assert_eq!(model.material, "materials/subject.json");
        assert_eq!(model.puppet.as_deref(), Some("models/subject_puppet.mdl"));
        assert!(model.unsupported_features.is_empty());
    }

    #[test]
    fn rejects_missing_material_and_escaping_paths() {
        assert!(parse_model_definition(r#"{"autosize":true}"#).is_err());
        assert!(parse_model_definition(r#"{"material":"../secret"}"#).is_err());
        assert!(parse_model_definition(r#"{"material":"safe","puppet":"/tmp/model"}"#).is_err());
    }

    #[test]
    fn reports_unknown_fields_deterministically() {
        let model = parse_model_definition(r#"{"zeta":1,"material":"materials/a.json","alpha":2}"#)
            .unwrap();
        assert_eq!(model.unsupported_features[0].path, "model.alpha");
        assert_eq!(model.unsupported_features[1].path, "model.zeta");
    }
}
