//! Sauvegardes au format de Telmi OS, pour pouvoir revenir à Telmi sans perte.
//!
//! - `/mnt/SDCARD/Saves/.storytellerState` : application ouverte à l'extinction ;
//! - `/mnt/SDCARD/Saves/Stories/<dossier>` : reprise d'une histoire précise.
//!
//! Les écritures passent par un fichier temporaire puis un renommage, pour
//! qu'une coupure de batterie ne laisse jamais un fichier tronqué.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{json, Value};

use crate::engine::Resume;

pub const APP_STORIES: i64 = 0;
pub const APP_MUSIC: i64 = 1;
pub const APP_NIGHT_MODE: i64 = 2;

/// Reprise d'une histoire, avec le dossier et la position dans l'audio.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StorySave {
    pub story_dir: String,
    /// Position dans l'audio de l'étape, en secondes.
    pub time: f64,
    pub resume: Resume,
}

impl StorySave {
    /// Vrai si la sauvegarde ne fait que pointer l'écran titre d'une histoire.
    pub fn is_title_only(&self) -> bool {
        self.resume.action_key.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NightSave {
    /// Chemins complets des audios de la liste de lecture.
    pub playlist: Vec<String>,
    pub index: usize,
    pub time: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicSave {
    pub index: usize,
    pub position: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SessionState {
    Stories(StorySave),
    /// Écran titre de l'application mode nuit.
    NightModeTitle(StorySave),
    NightModePlaying(NightSave),
    Music(MusicSave),
    /// Application connue mais sans état exploitable.
    App(i64),
}

impl SessionState {
    pub fn app(&self) -> i64 {
        match self {
            SessionState::Stories(_) => APP_STORIES,
            SessionState::NightModeTitle(_) | SessionState::NightModePlaying(_) => APP_NIGHT_MODE,
            SessionState::Music(_) => APP_MUSIC,
            SessionState::App(app) => *app,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            SessionState::Stories(s) => story_json(APP_STORIES, s),
            SessionState::NightModeTitle(s) => story_json(APP_NIGHT_MODE, s),
            SessionState::NightModePlaying(n) => json!({
                // Telmi range la lecture nocturne sous l'application Histoires.
                "app": APP_STORIES,
                "nightMode": true,
                "playlist": n.playlist,
                "storyIndex": n.index,
                "storyTime": n.time,
            }),
            SessionState::Music(m) => json!({
                "app": APP_MUSIC,
                "musicIndex": m.index,
                "musicPosition": m.position,
            }),
            SessionState::App(app) => json!({ "app": app }),
        }
    }

    pub fn from_json(value: &Value) -> Option<SessionState> {
        let app = value.get("app")?.as_f64()? as i64;
        if value.get("nightMode").and_then(Value::as_bool) == Some(true) {
            let playlist: Vec<String> = value
                .get("playlist")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            return Some(SessionState::NightModePlaying(NightSave {
                index: number(value, "storyIndex").max(0.0) as usize,
                time: number(value, "storyTime").max(0.0),
                playlist,
            }));
        }
        Some(match app {
            APP_MUSIC => SessionState::Music(MusicSave {
                index: number(value, "musicIndex").max(0.0) as usize,
                position: number(value, "musicPosition").max(0.0),
            }),
            APP_STORIES | APP_NIGHT_MODE => match story_from_json(value) {
                Some(s) if app == APP_STORIES => SessionState::Stories(s),
                Some(s) => SessionState::NightModeTitle(s),
                None => SessionState::App(app),
            },
            other => SessionState::App(other),
        })
    }
}

fn story_json(app: i64, s: &StorySave) -> Value {
    json!({
        "app": app,
        "storyPath": s.story_dir,
        "storyActionKey": s.resume.action_key,
        "storyActionOptionIndex": s.resume.option_index,
        "storyTime": s.time,
        "storyPlayingTime": s.resume.playing_time,
        "inventory": s.resume.inventory,
    })
}

fn story_from_json(value: &Value) -> Option<StorySave> {
    let story_dir = value.get("storyPath")?.as_str()?.to_owned();
    let inventory = value
        .get("inventory")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|v| v.as_f64().unwrap_or(0.0) as i32).collect())
        .unwrap_or_default();
    Some(StorySave {
        story_dir,
        time: number(value, "storyTime").max(0.0),
        resume: Resume {
            action_key: value
                .get("storyActionKey")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            option_index: number(value, "storyActionOptionIndex") as i32,
            playing_time: number(value, "storyPlayingTime") as i64,
            inventory,
        },
    })
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(0.0)
}

/// Lit une sauvegarde ; tout fichier absent ou illisible vaut `None`.
pub fn read_session(path: &Path) -> Option<SessionState> {
    let bytes = fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    SessionState::from_json(&value)
}

pub fn write_session(path: &Path, state: &SessionState) -> std::io::Result<()> {
    write_atomic(path, state.to_json().to_string().as_bytes())
}

/// Écrit `data` dans un fichier temporaire synchronisé, puis le renomme.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = Path::new(&tmp);
    {
        let mut file = fs::File::create(tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_telmi_story_save() {
        // Fichier tel qu'écrit par `file_save` de Telmi OS.
        let raw = r#"{"app":0, "storyPath":"02_00_Histoire_fffffb-col-02", "storyActionKey":"menu",
            "storyActionOptionIndex":3, "storyTime":12.500000, "storyPlayingTime":40, "inventory":[]}"#;
        let state = SessionState::from_json(&serde_json::from_str(raw).unwrap()).unwrap();
        let SessionState::Stories(s) = &state else { panic!("{state:?}") };
        assert_eq!(s.story_dir, "02_00_Histoire_fffffb-col-02");
        assert_eq!(s.resume.option_index, 3);
        assert_eq!(s.time, 12.5);
        assert_eq!(SessionState::from_json(&state.to_json()).unwrap(), state);
    }

    #[test]
    fn reads_telmi_night_mode_save() {
        let raw = r#"{"app":0, "nightMode":true, "playlist":["/a.mp3","/b.mp3","","","","","","","","","","","","","",""],
            "storyIndex":1, "storyTime":3.0}"#;
        let state = SessionState::from_json(&serde_json::from_str(raw).unwrap()).unwrap();
        let SessionState::NightModePlaying(n) = state else { panic!() };
        assert_eq!(n.playlist, ["/a.mp3", "/b.mp3"]);
        assert_eq!(n.index, 1);
    }

    #[test]
    fn tolerates_garbage() {
        assert_eq!(SessionState::from_json(&json!({"app": "x"})), None);
        assert_eq!(SessionState::from_json(&json!({"app": 0})), Some(SessionState::App(0)));
        assert_eq!(
            SessionState::from_json(&json!({"app": 1, "musicIndex": -4})),
            Some(SessionState::Music(MusicSave { index: 0, position: 0.0 }))
        );
    }

    #[test]
    fn writes_atomically() {
        let dir = std::env::temp_dir().join(format!("telmi-save-{}", std::process::id()));
        let path = dir.join("Stories").join("x");
        let state = SessionState::Music(MusicSave { index: 4, position: 1.5 });
        write_session(&path, &state).unwrap();
        assert_eq!(read_session(&path), Some(state));
        assert!(!dir.join("Stories").join("x.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
