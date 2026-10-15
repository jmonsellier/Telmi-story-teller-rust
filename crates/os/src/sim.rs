//! Outils du simulateur Mac (absents du binaire de la console).
//!
//! - `TELMI_SCRIPT="A,wait:2000,Right,A,Menu,Quit"` rejoue des appuis de boutons ;
//! - `TELMI_SNAPSHOTS=<dossier>` enregistre chaque image affichée en PNG.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::input::{Button, Event};

enum Step {
    Press(Button),
    Wait(Duration),
    Quit,
}

pub struct Script {
    steps: VecDeque<Step>,
    resume_at: Instant,
}

impl Script {
    pub fn from_env() -> Option<Script> {
        let raw = std::env::var("TELMI_SCRIPT").ok()?;
        let steps = raw
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|token| {
                if let Some(ms) = token.strip_prefix("wait:") {
                    return Step::Wait(Duration::from_millis(ms.parse().expect("wait:<ms>")));
                }
                Step::Press(match token {
                    "Up" => Button::Up,
                    "Down" => Button::Down,
                    "Left" => Button::Left,
                    "Right" => Button::Right,
                    "A" => Button::A,
                    "B" => Button::B,
                    "X" => Button::X,
                    "Y" => Button::Y,
                    "Start" => Button::Start,
                    "Select" => Button::Select,
                    "Menu" => Button::Menu,
                    "VolumeUp" => Button::VolumeUp,
                    "VolumeDown" => Button::VolumeDown,
                    "Quit" => return Step::Quit,
                    other => panic!("bouton inconnu dans TELMI_SCRIPT : {other}"),
                })
            })
            .collect();
        Some(Script { steps, resume_at: Instant::now() + Duration::from_millis(500) })
    }

    pub fn wait(&mut self, timeout_ms: i32, events: &mut Vec<Event>) {
        let now = Instant::now();
        if now < self.resume_at {
            std::thread::sleep((self.resume_at - now).min(Duration::from_millis(timeout_ms as u64)));
            return;
        }
        // Petite pause entre deux appuis pour laisser l'anti-rebond passer.
        self.resume_at = now + Duration::from_millis(120);
        match self.steps.pop_front() {
            Some(Step::Press(button)) => events.extend([Event::Pressed(button), Event::Released(button)]),
            Some(Step::Wait(d)) => self.resume_at = now + d,
            Some(Step::Quit) | None => events.push(Event::Quit),
        }
    }
}

pub struct Snapshots {
    dir: PathBuf,
    count: usize,
}

impl Snapshots {
    pub fn from_env() -> Option<Snapshots> {
        let dir = PathBuf::from(std::env::var_os("TELMI_SNAPSHOTS")?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Snapshots { dir, count: 0 })
    }

    pub fn save(&mut self, pixels: &[u32]) {
        self.count += 1;
        let path = self.dir.join(format!("{:04}.png", self.count));
        let Ok(file) = std::fs::File::create(&path) else { return };
        let mut encoder = png::Encoder::new(file, crate::sdl::WIDTH as u32, crate::sdl::HEIGHT as u32);
        encoder.set_color(png::ColorType::Rgb);
        let data: Vec<u8> = pixels.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
        if let Ok(mut writer) = encoder.write_header() {
            let _ = writer.write_image_data(&data);
        }
    }
}
