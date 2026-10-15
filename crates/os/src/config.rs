//! Chemins de la carte SD et paramètres de Telmi Sync (`Saves/.parameters`).

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Arborescence de la carte SD. `TELMI_ROOT` la remplace dans le simulateur.
#[derive(Clone, Debug)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn from_env() -> Paths {
        let root = std::env::var_os("TELMI_ROOT").map_or_else(|| PathBuf::from("/mnt/SDCARD"), PathBuf::from);
        Paths { root }
    }

    pub fn res(&self) -> PathBuf {
        self.root.join(".tmp_update/res")
    }

    pub fn res_file(&self, name: &str) -> PathBuf {
        self.res().join(name)
    }

    pub fn stories(&self) -> PathBuf {
        self.root.join("Stories")
    }

    pub fn music(&self) -> PathBuf {
        self.root.join("Music")
    }

    pub fn session(&self) -> PathBuf {
        self.root.join("Saves/.storytellerState")
    }

    pub fn story_saves(&self) -> PathBuf {
        self.root.join("Saves/Stories")
    }

    pub fn parameters(&self) -> PathBuf {
        self.root.join("Saves/.parameters")
    }

    pub fn clock_file(&self) -> PathBuf {
        self.root.join("Saves/CurrentProfile/saves/currentTime.txt")
    }

    pub fn log_file(&self) -> PathBuf {
        self.root.join(".tmp_update/logs/storyTeller-rs.log")
    }
}

/// Réglages choisis dans Telmi Sync, avec les valeurs par défaut de Telmi OS.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    pub audio_volume_startup: f64,
    pub audio_volume_max: f64,
    pub screen_brightness_startup: f64,
    pub screen_brightness_max: f64,
    pub screen_on_inactivity: i64,
    pub screen_off_inactivity: i64,
    pub music_inactivity: i64,
    pub story_display_tiles: bool,
    pub story_disable_night_mode: bool,
    pub story_disable_timeline: bool,
    pub music_disable_repeat_modes: bool,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            audio_volume_startup: 0.3,
            audio_volume_max: 0.6,
            screen_brightness_startup: 0.3,
            screen_brightness_max: 0.6,
            screen_on_inactivity: 120,
            screen_off_inactivity: 300,
            music_inactivity: 3600,
            story_display_tiles: false,
            story_disable_night_mode: false,
            story_disable_timeline: false,
            music_disable_repeat_modes: false,
        }
    }
}

impl Parameters {
    pub fn load(path: &Path) -> Parameters {
        let mut p = Parameters::default();
        let Some(json) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else {
            return p;
        };
        let ratio = |key: &str, target: &mut f64| {
            if let Some(v) = json.get(key).and_then(Value::as_f64).filter(|v| (0.0..=1.0).contains(v)) {
                *target = v;
            }
        };
        ratio("audioVolumeStartup", &mut p.audio_volume_startup);
        ratio("audioVolumeMax", &mut p.audio_volume_max);
        ratio("screenBrightnessStartup", &mut p.screen_brightness_startup);
        ratio("screenBrightnessMax", &mut p.screen_brightness_max);
        let seconds = |key: &str, target: &mut i64| {
            if let Some(v) = json.get(key).and_then(Value::as_f64).filter(|v| *v > 0.0) {
                *target = v as i64;
            }
        };
        seconds("screenOnInactivityTime", &mut p.screen_on_inactivity);
        seconds("screenOffInactivityTime", &mut p.screen_off_inactivity);
        seconds("musicInactivityTime", &mut p.music_inactivity);
        let flag = |key: &str, target: &mut bool| {
            if let Some(v) = json.get(key).and_then(Value::as_bool) {
                *target = v;
            }
        };
        flag("storyDisplayTiles", &mut p.story_display_tiles);
        flag("storyDisableNightMode", &mut p.story_disable_night_mode);
        flag("storyDisableTimeline", &mut p.story_disable_timeline);
        flag("musicDisableRepeatModes", &mut p.music_disable_repeat_modes);
        p
    }

    fn scale(ratio: f64, max: i32) -> i32 {
        (ratio * max as f64 + 0.5) as i32
    }

    pub fn volume_startup(&self) -> i32 {
        Self::scale(self.audio_volume_startup, crate::hw::VOLUME_MAX)
    }

    pub fn volume_max(&self) -> i32 {
        Self::scale(self.audio_volume_max, crate::hw::VOLUME_MAX)
    }

    pub fn brightness_startup(&self) -> i32 {
        Self::scale(self.screen_brightness_startup, crate::hw::BRIGHTNESS_MAX)
    }

    pub fn brightness_max(&self) -> i32 {
        Self::scale(self.screen_brightness_max, crate::hw::BRIGHTNESS_MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_telmi_sync_parameters() {
        let dir = std::env::temp_dir().join(format!("telmi-params-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".parameters");
        std::fs::write(
            &path,
            r#"{"audioVolumeMax": 0.8, "screenOnInactivityTime": 60, "storyDisplayTiles": true,
                "storyDisableNightMode": null, "audioVolumeStartup": 7}"#,
        )
        .unwrap();
        let p = Parameters::load(&path);
        assert_eq!(p.volume_max(), 20);
        assert_eq!(p.volume_startup(), 8, "valeur hors bornes ignorée");
        assert_eq!(p.screen_on_inactivity, 60);
        assert!(p.story_display_tiles && !p.story_disable_night_mode);
        assert_eq!(Parameters::load(&dir.join("absent")), Parameters::default());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
