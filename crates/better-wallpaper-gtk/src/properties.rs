//! Native scene property editors. Wire values remain strings for every type.
use std::{collections::BTreeMap, rc::Rc};

use gtk::{gdk, prelude::*};
use serde_json::Value;

use crate::i18n::tr;

#[derive(Clone)]
pub enum PropertyInput {
    Text(gtk::Entry),
    Bool(gtk::CheckButton),
    Number(gtk::SpinButton),
    Choice(gtk::DropDown, Vec<String>),
    Color(gtk::ColorButton),
}

impl PropertyInput {
    pub fn new(
        property: &Value,
        value: &str,
        window: &gtk::ApplicationWindow,
    ) -> (Self, gtk::Widget) {
        let kind = &property["prop_type"];
        match kind["kind"].as_str().unwrap_or_default() {
            "bool" if matches!(value, "true" | "false") => {
                let input = gtk::CheckButton::new();
                input.set_active(value == "true");
                (Self::Bool(input.clone()), input.upcast())
            }
            "slider" if value.parse::<f64>().is_ok_and(f64::is_finite) => {
                let value = value.parse::<f64>().unwrap();
                let min = kind["min"]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .unwrap_or(0.0)
                    .min(value);
                let max = kind["max"]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .unwrap_or(100.0)
                    .max(value)
                    .max(min);
                let step = kind["step"]
                    .as_f64()
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .unwrap_or(0.01);
                let input = gtk::SpinButton::with_range(min, max, step);
                input.set_digits(6);
                input.set_value(value);
                (Self::Number(input.clone()), input.upcast())
            }
            "combo" => {
                let mut labels = Vec::new();
                let mut values = Vec::new();
                for option in kind["options"].as_array().into_iter().flatten() {
                    if let (Some(label), Some(value)) = (option[0].as_str(), option[1].as_str()) {
                        labels.push(label.to_owned());
                        values.push(value.to_owned());
                    }
                }
                // Keep values authored by a newer project version selectable.
                if !values.iter().any(|v| v == value) {
                    labels.push(value.to_owned());
                    values.push(value.to_owned());
                }
                let labels: Vec<_> = labels.iter().map(String::as_str).collect();
                let input = gtk::DropDown::from_strings(&labels);
                input.set_selected(values.iter().position(|v| v == value).unwrap_or(0) as u32);
                (Self::Choice(input.clone(), values), input.upcast())
            }
            "color" if parse_color(value).is_some() => {
                let input = gtk::ColorButton::with_rgba(&parse_color(value).unwrap());
                input.set_use_alpha(false);
                (Self::Color(input.clone()), input.upcast())
            }
            _ => {
                let input = gtk::Entry::builder().text(value).hexpand(true).build();
                if kind["kind"] != "file" {
                    return (Self::Text(input.clone()), input.upcast());
                }
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let browse = gtk::Button::with_label(tr("Browse…"));
                content.append(&input);
                content.append(&browse);
                let parent = window.downgrade();
                let entry = input.downgrade();
                browse.connect_clicked(move |_| {
                    let (Some(parent), Some(entry)) = (parent.upgrade(), entry.upgrade()) else {
                        return;
                    };
                    let chooser = gtk::FileChooserNative::new(
                        Some(tr("Choose file")),
                        Some(&parent),
                        gtk::FileChooserAction::Open,
                        Some(tr("Choose")),
                        Some(tr("Cancel")),
                    );
                    chooser.connect_response(move |dialog, response| {
                        if response == gtk::ResponseType::Accept
                            && let Some(path) = dialog.file().and_then(|file| file.path())
                        {
                            entry.set_text(&path.to_string_lossy());
                        }
                        dialog.destroy();
                    });
                    chooser.show();
                });
                (Self::Text(input), content.upcast())
            }
        }
    }

    pub fn read(&self) -> String {
        match self {
            Self::Text(input) => input.text().to_string(),
            Self::Bool(input) => input.is_active().to_string(),
            Self::Number(input) => input.value().to_string(),
            Self::Choice(input, values) => values
                .get(input.selected() as usize)
                .cloned()
                .unwrap_or_default(),
            Self::Color(input) => {
                let color = input.rgba();
                format!("{} {} {}", color.red(), color.green(), color.blue())
            }
        }
    }

    pub fn connect_changed(&self, callback: Rc<dyn Fn()>) {
        match self {
            Self::Text(input) => {
                input.connect_changed(move |_| callback());
            }
            Self::Bool(input) => {
                input.connect_toggled(move |_| callback());
            }
            Self::Number(input) => {
                input.connect_value_changed(move |_| callback());
            }
            Self::Choice(input, _) => {
                input.connect_selected_notify(move |_| callback());
            }
            Self::Color(input) => {
                input.connect_color_set(move |_| callback());
            }
        }
    }
}

pub fn property_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

fn parse_color(value: &str) -> Option<gdk::RGBA> {
    let channels: Vec<f32> = value
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if channels.len() != 3
        || !channels
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    {
        return None;
    }
    Some(gdk::RGBA::new(channels[0], channels[1], channels[2], 1.0))
}

/// Match the former browser consumer's supported `key.value ==/!= literal` form.
/// Unsupported expressions stay visible rather than hiding usable controls.
pub fn property_visible(condition: Option<&str>, values: &BTreeMap<String, String>) -> bool {
    let Some(condition) = condition else {
        return true;
    };
    let (left, expected, equal) = if let Some((left, right)) = condition.split_once("==") {
        (left, right, true)
    } else if let Some((left, right)) = condition.split_once("!=") {
        (left, right, false)
    } else {
        return true;
    };
    let Some(key) = left.trim().strip_suffix(".value") else {
        return true;
    };
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    {
        return true;
    }
    let expected = expected.trim();
    let expected = expected
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            expected
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(expected);
    (values.get(key).map(String::as_str).unwrap_or_default() == expected) == equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_use_default_or_overridden_string_values() {
        let mut values = BTreeMap::from([
            ("enabled".into(), "false".into()),
            ("mode".into(), "slow mode".into()),
        ]);
        assert!(!property_visible(Some("enabled.value == true"), &values));
        values.insert("enabled".into(), "true".into());
        assert!(property_visible(Some("enabled.value == true"), &values));
        assert!(property_visible(Some("mode.value == 'slow mode'"), &values));
        assert!(!property_visible(
            Some("mode.value != \"slow mode\""),
            &values
        ));
        assert!(property_visible(Some("unsupported expression"), &values));
        assert!(property_visible(None, &values));
    }
}
