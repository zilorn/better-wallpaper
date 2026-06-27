use std::{fs, path::PathBuf};

use crate::BackendKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopKind {
    Niri,
    Kde,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopDetection {
    pub kind: DesktopKind,
    pub evidence: String,
    pub candidates: Vec<String>,
}

pub trait Environment {
    fn var(&self, key: &str) -> Option<String>;
    fn runtime_entries(&self) -> Vec<PathBuf>;
}

pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }
    fn runtime_entries(&self) -> Vec<PathBuf> {
        self.var("XDG_RUNTIME_DIR")
            .and_then(|dir| fs::read_dir(dir).ok())
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect()
    }
}

pub fn detect_desktop(env: &impl Environment) -> DesktopDetection {
    let mut candidates = Vec::new();
    for key in [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ] {
        if let Some(value) = env.var(key) {
            let tokens: Vec<_> = value
                .split([':', ';'])
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_lowercase)
                .collect();
            candidates.extend(tokens.iter().cloned());
            if let Some(kind) = tokens.iter().find_map(|token| classify(token)) {
                return DesktopDetection {
                    kind,
                    evidence: format!("{key}={value}"),
                    candidates,
                };
            }
        }
    }
    if let Some(socket) = env.var("NIRI_SOCKET") {
        candidates.push("niri".into());
        return DesktopDetection {
            kind: DesktopKind::Niri,
            evidence: format!("NIRI_SOCKET={socket}"),
            candidates,
        };
    }
    if env.runtime_entries().iter().any(|path| {
        path.file_name().is_some_and(|name| {
            name.to_string_lossy().starts_with("niri.") && name.to_string_lossy().ends_with(".sock")
        })
    }) {
        candidates.push("niri".into());
        return DesktopDetection {
            kind: DesktopKind::Niri,
            evidence: "XDG_RUNTIME_DIR/niri.*.sock".into(),
            candidates,
        };
    }
    DesktopDetection {
        kind: DesktopKind::Unknown,
        evidence: "未找到受支持的桌面环境标识".into(),
        candidates,
    }
}

fn classify(token: &str) -> Option<DesktopKind> {
    if token == "niri" {
        Some(DesktopKind::Niri)
    } else if token == "kde" || token == "plasma" || token.starts_with("plasma-") {
        Some(DesktopKind::Kde)
    } else {
        None
    }
}

pub fn select_backend(
    cli: Option<BackendKind>,
    configured: BackendKind,
    desktop: DesktopKind,
) -> BackendKind {
    cli.filter(|v| *v != BackendKind::Auto)
        .or_else(|| (configured != BackendKind::Auto).then_some(configured))
        .unwrap_or(match desktop {
            DesktopKind::Niri => BackendKind::Niri,
            DesktopKind::Kde => BackendKind::Kde,
            DesktopKind::Unknown => BackendKind::Headless,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeEnv {
        vars: BTreeMap<String, String>,
        entries: Vec<PathBuf>,
    }
    impl Environment for FakeEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
        fn runtime_entries(&self) -> Vec<PathBuf> {
            self.entries.clone()
        }
    }
    fn env(values: &[(&str, &str)]) -> FakeEnv {
        FakeEnv {
            vars: values
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            entries: vec![],
        }
    }

    #[test]
    fn respects_variable_priority() {
        let result = detect_desktop(&env(&[
            ("XDG_CURRENT_DESKTOP", "KDE"),
            ("XDG_SESSION_DESKTOP", "niri"),
        ]));
        assert_eq!(result.kind, DesktopKind::Kde);
        assert!(result.evidence.starts_with("XDG_CURRENT_DESKTOP"));
    }
    #[test]
    fn accepts_case_and_compound_tokens() {
        let result = detect_desktop(&env(&[("XDG_CURRENT_DESKTOP", "GNOME;NiRi")]));
        assert_eq!(result.kind, DesktopKind::Niri);
        assert_eq!(result.candidates, ["gnome", "niri"]);
    }
    #[test]
    fn falls_back_to_niri_socket_variable() {
        assert_eq!(
            detect_desktop(&env(&[("NIRI_SOCKET", "/run/user/1/niri.sock")])).kind,
            DesktopKind::Niri
        );
    }
    #[test]
    fn falls_back_to_runtime_socket() {
        let fake = FakeEnv {
            entries: vec!["/run/user/1/niri.wayland-1.sock".into()],
            ..Default::default()
        };
        assert_eq!(detect_desktop(&fake).kind, DesktopKind::Niri);
    }
    #[test]
    fn reports_unknown_desktop() {
        assert_eq!(
            detect_desktop(&env(&[("XDG_CURRENT_DESKTOP", "sway")])).kind,
            DesktopKind::Unknown
        );
    }
    #[test]
    fn backend_precedence_is_cli_config_then_detection() {
        assert_eq!(
            select_backend(
                Some(BackendKind::Headless),
                BackendKind::Kde,
                DesktopKind::Niri
            ),
            BackendKind::Headless
        );
        assert_eq!(
            select_backend(None, BackendKind::Kde, DesktopKind::Niri),
            BackendKind::Kde
        );
        assert_eq!(
            select_backend(None, BackendKind::Auto, DesktopKind::Niri),
            BackendKind::Niri
        );
        assert_eq!(
            select_backend(None, BackendKind::Auto, DesktopKind::Unknown),
            BackendKind::Headless
        );
    }
}
