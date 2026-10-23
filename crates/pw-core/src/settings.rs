//! Versioned user settings (PRD 03 "Design system", INTERACTION_CONTRACT "Theme and settings").
//!
//! Settings are untrusted input when loaded: every field is validated and clamped, unknown fields
//! are ignored, older schema versions are migrated, and a missing or corrupt file recovers to safe
//! defaults while preserving the bad file for diagnosis. Saving is atomic.

use crate::error::{PwError, PwResult};
use crate::output::{write_atomic_bytes, CollisionPolicy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SETTINGS_VERSION: u32 = 1;
pub const MAX_RECENT_FILES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    #[default]
    Compact,
    Comfortable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MotionPreference {
    /// Follow the operating system's reduced-motion setting.
    #[default]
    System,
    Reduced,
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub version: u32,
    pub theme: Theme,
    /// `#rrggbb`.
    pub accent: String,
    pub density: Density,
    /// UI text scale, 0.8–2.0.
    pub ui_scale: f64,
    pub motion: MotionPreference,
    /// Most-recent-first list of file paths (paths only, never content).
    pub recent_files: Vec<String>,
    /// Command ID → user shortcut override (empty string = unbound).
    pub shortcuts: BTreeMap<String, String>,
    pub restore_session: bool,
    /// Read aloud: the chosen voice id (the system default when `None`).
    #[serde(default)]
    pub read_voice: Option<String>,
    /// Read aloud speed, 0.5–2.0.
    #[serde(default = "normal_rate")]
    pub read_rate: f64,
}

fn normal_rate() -> f64 {
    1.0
}

pub const DEFAULT_ACCENT: &str = "#2f6fde";

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: SETTINGS_VERSION,
            theme: Theme::System,
            accent: DEFAULT_ACCENT.into(),
            density: Density::Compact,
            ui_scale: 1.0,
            motion: MotionPreference::System,
            recent_files: Vec::new(),
            shortcuts: BTreeMap::new(),
            restore_session: true,
            read_voice: None,
            read_rate: 1.0,
        }
    }
}

/// What happened while loading — surfaced to the user when it is not `Loaded`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum LoadOutcome {
    Loaded,
    Missing,
    Migrated {
        from: u32,
    },
    /// Unreadable/corrupt file: defaults used, original kept at `backup`.
    RecoveredDefaults {
        backup: Option<String>,
    },
    /// Some fields were invalid and were reset.
    Repaired {
        fields: Vec<String>,
    },
    /// Written by a newer PageWeave: values read best-effort, file not overwritten until the user saves.
    NewerVersion {
        found: u32,
    },
}

pub fn is_valid_accent(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl Settings {
    /// Validate a JSON value field by field. Invalid fields fall back to defaults and are reported.
    pub fn from_value(v: &Value) -> (Settings, Vec<String>) {
        let d = Settings::default();
        let mut bad = Vec::new();
        let obj = match v.as_object() {
            Some(o) => o,
            None => return (d, vec!["<root>".into()]),
        };
        fn field<T: for<'a> Deserialize<'a>>(
            o: &serde_json::Map<String, Value>,
            k: &str,
            bad: &mut Vec<String>,
        ) -> Option<T> {
            let raw = o.get(k)?;
            match serde_json::from_value(raw.clone()) {
                Ok(v) => Some(v),
                Err(_) => {
                    bad.push(k.to_string());
                    None
                }
            }
        }
        let mut s = d.clone();
        if let Some(t) = field(obj, "theme", &mut bad) {
            s.theme = t;
        }
        if let Some(a) = field::<String>(obj, "accent", &mut bad) {
            if is_valid_accent(&a) {
                s.accent = a.to_ascii_lowercase();
            } else {
                bad.push("accent".into());
            }
        }
        if let Some(x) = field(obj, "density", &mut bad) {
            s.density = x;
        }
        if let Some(x) = field::<f64>(obj, "ui_scale", &mut bad) {
            if x.is_finite() && (0.8..=2.0).contains(&x) {
                s.ui_scale = x;
            } else {
                bad.push("ui_scale".into());
            }
        }
        if let Some(x) = field(obj, "motion", &mut bad) {
            s.motion = x;
        }
        if let Some(list) = field::<Vec<String>>(obj, "recent_files", &mut bad) {
            let mut seen = std::collections::HashSet::new();
            s.recent_files = list
                .into_iter()
                .filter(|p| !p.trim().is_empty() && p.len() <= 32_767 && !p.contains('\0'))
                .filter(|p| seen.insert(p.clone()))
                .take(MAX_RECENT_FILES)
                .collect();
        }
        if let Some(map) = field::<BTreeMap<String, String>>(obj, "shortcuts", &mut bad) {
            s.shortcuts = map.into_iter().filter(|(k, v)| k.len() <= 128 && v.len() <= 64).take(500).collect();
        }
        if let Some(x) = field(obj, "restore_session", &mut bad) {
            s.restore_session = x;
        }
        if let Some(x) = field::<Option<String>>(obj, "read_voice", &mut bad) {
            match x {
                Some(v) if v.is_empty() || v.len() > 512 || v.contains('\0') => bad.push("read_voice".into()),
                v => s.read_voice = v,
            }
        }
        if let Some(x) = field::<f64>(obj, "read_rate", &mut bad) {
            if x.is_finite() && (0.5..=2.0).contains(&x) {
                s.read_rate = x;
            } else {
                bad.push("read_rate".into());
            }
        }
        (s, bad)
    }

    /// Record a file as most recently used.
    pub fn touch_recent(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(MAX_RECENT_FILES);
    }
}

/// Migrate an older on-disk shape to the current version's shape. Version 0 = pre-release
/// files that used `"dark_mode": bool` instead of `"theme"`.
fn migrate(mut v: Value, from: u32) -> Value {
    if from == 0 {
        if let Some(o) = v.as_object_mut() {
            if let Some(dark) = o.remove("dark_mode").and_then(|d| d.as_bool()) {
                o.insert("theme".into(), Value::String(if dark { "dark" } else { "light" }.into()));
            }
        }
    }
    v
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        SettingsStore { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> (Settings, LoadOutcome) {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), LoadOutcome::Missing),
            Err(_) => return (Settings::default(), LoadOutcome::RecoveredDefaults { backup: None }),
        };
        let value: Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => {
                let backup = self.backup_corrupt();
                return (Settings::default(), LoadOutcome::RecoveredDefaults { backup });
            }
        };
        let found = value.get("version").and_then(|v| v.as_u64()).map(|v| v as u32).unwrap_or(0);
        let value = if found < SETTINGS_VERSION { migrate(value, found) } else { value };
        let (s, bad) = Settings::from_value(&value);
        let outcome = if found > SETTINGS_VERSION {
            LoadOutcome::NewerVersion { found }
        } else if found < SETTINGS_VERSION {
            LoadOutcome::Migrated { from: found }
        } else if !bad.is_empty() {
            LoadOutcome::Repaired { fields: bad }
        } else {
            LoadOutcome::Loaded
        };
        (s, outcome)
    }

    fn backup_corrupt(&self) -> Option<String> {
        let backup = self.path.with_extension("corrupt.json");
        fs::rename(&self.path, &backup).ok()?;
        Some(backup.file_name()?.to_string_lossy().into_owned())
    }

    pub fn save(&self, s: &Settings) -> PwResult<()> {
        let mut s = s.clone();
        s.version = SETTINGS_VERSION;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| PwError::from_io(&e, "settings folder"))?;
        }
        let data = serde_json::to_vec_pretty(&s)
            .map_err(|_| PwError::internal("settings.serialize", "Could not save settings."))?;
        write_atomic_bytes(&self.path, CollisionPolicy::Replace, &[], &data, |p| {
            let back: Value = serde_json::from_slice(&fs::read(p).map_err(|e| PwError::from_io(&e, "settings file"))?)
                .map_err(|_| PwError::internal("settings.verify", "Settings verification failed."))?;
            let (_, bad) = Settings::from_value(&back);
            if bad.is_empty() {
                Ok(())
            } else {
                Err(PwError::internal("settings.verify", "Settings verification failed."))
            }
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (s, o) = SettingsStore::new(dir.path().join("settings.json")).load();
        assert_eq!(s, Settings::default());
        assert_eq!(o, LoadOutcome::Missing);
    }

    #[test]
    fn round_trip_persists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(dir.path().join("cfg").join("settings.json"));
        let mut s = Settings {
            theme: Theme::Dark,
            accent: "#aa3300".into(),
            density: Density::Comfortable,
            ui_scale: 1.25,
            motion: MotionPreference::Reduced,
            ..Default::default()
        };
        s.shortcuts.insert("file.open".into(), "Ctrl+Shift+O".into());
        s.touch_recent("C:\\docs\\a.pdf");
        store.save(&s).expect("save");
        let (back, o) = store.load();
        assert_eq!(o, LoadOutcome::Loaded);
        assert_eq!(back, s);
    }

    #[test]
    fn corrupt_file_recovers_and_keeps_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("settings.json");
        fs::write(&p, b"{ not json").expect("seed");
        let (s, o) = SettingsStore::new(&p).load();
        assert_eq!(s, Settings::default());
        assert_eq!(o, LoadOutcome::RecoveredDefaults { backup: Some("settings.corrupt.json".into()) });
        assert_eq!(fs::read(dir.path().join("settings.corrupt.json")).expect("backup"), b"{ not json");
    }

    #[test]
    fn read_aloud_voice_and_speed_persist_and_bad_values_are_repaired() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("settings.json");
        fs::write(&p, r#"{"version":1,"read_voice":"slt","read_rate":1.5}"#).expect("seed");
        let (s, _) = SettingsStore::new(&p).load();
        assert_eq!((s.read_voice.as_deref(), s.read_rate), (Some("slt"), 1.5));
        fs::write(&p, r#"{"version":1,"read_voice":"","read_rate":7}"#).expect("seed");
        let (s, o) = SettingsStore::new(&p).load();
        assert_eq!((s.read_voice, s.read_rate), (None, 1.0));
        assert!(matches!(o, LoadOutcome::Repaired { fields } if fields == ["read_voice", "read_rate"]));
        // Older files without the fields get the defaults.
        fs::write(&p, r#"{"version":1,"theme":"dark"}"#).expect("seed");
        let (s, _) = SettingsStore::new(&p).load();
        assert_eq!((s.read_voice, s.read_rate), (None, 1.0));
    }

    #[test]
    fn invalid_fields_are_repaired_individually() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("settings.json");
        fs::write(
            &p,
            r#"{"version":1,"theme":"neon","accent":"red","ui_scale":9,"density":"comfortable","unknown":1}"#,
        )
        .expect("seed");
        let (s, o) = SettingsStore::new(&p).load();
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.accent, DEFAULT_ACCENT);
        assert_eq!(s.ui_scale, 1.0);
        assert_eq!(s.density, Density::Comfortable, "valid fields survive");
        match o {
            LoadOutcome::Repaired { fields } => {
                assert!(
                    fields.contains(&"theme".into())
                        && fields.contains(&"accent".into())
                        && fields.contains(&"ui_scale".into())
                )
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn migrates_version_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("settings.json");
        fs::write(&p, r#"{"dark_mode":true}"#).expect("seed");
        let (s, o) = SettingsStore::new(&p).load();
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(o, LoadOutcome::Migrated { from: 0 });
    }

    #[test]
    fn newer_version_is_flagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("settings.json");
        fs::write(&p, r#"{"version":99,"theme":"light"}"#).expect("seed");
        let (s, o) = SettingsStore::new(&p).load();
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(o, LoadOutcome::NewerVersion { found: 99 });
    }

    #[test]
    fn recent_files_are_bounded_and_deduplicated() {
        let mut s = Settings::default();
        for i in 0..30 {
            s.touch_recent(&format!("/f{i}.pdf"));
        }
        s.touch_recent("/f25.pdf");
        assert_eq!(s.recent_files.len(), MAX_RECENT_FILES);
        assert_eq!(s.recent_files[0], "/f25.pdf");
        assert_eq!(s.recent_files.iter().filter(|p| *p == "/f25.pdf").count(), 1);
    }
}
