//! Application Histoires (et mode nuit), portage de `stories_reader.h`.
//!
//! Différences voulues avec Telmi OS :
//! - l'histoire est lue en flux vers une forme compacte (voir `telmi-story`) ;
//! - une histoire illisible ramène au titre avec un message au lieu de figer ;
//! - seuls les dossiers contenant un `nodes.json` sont listés.

use std::path::{Path, PathBuf};

use telmi_story::save::{self, NightSave, SessionState, StorySave};
use telmi_story::{load_story, Player, Resume, Step};

use crate::gfx::Color;
use crate::log;
use crate::text::{Align, Style, Weight};
use crate::ui::{format_time, Ctx, TEXT_16};

const NIGHT_PLAYLIST_MAX: usize = 16;
const TILES_PER_PAGE: usize = 9;
/// Secondes d'affichage de la frise après un appui pendant une lecture sans image.
const TIMELINE_SECS: i64 = 5;

/// Action à déclencher quand l'audio en cours se termine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hook {
    None,
    Autoplay,
    NightNext,
}

pub struct Stories {
    night_mode: bool,
    list: Vec<String>,
    has_save: Vec<bool>,
    index: usize,
    tiles: bool,
    /// Histoire ouverte (`None` : écran titre).
    player: Option<Player>,
    autoplay: bool,
    ok_action: bool,
    screen_enabled: bool,
    show_timeline: bool,
    timeline_second: i64,
    timeline_drawn: i64,
    timeline_until: i64,
    hook: Hook,
    night_playing: bool,
    night_index: usize,
    night_playlist: Vec<PathBuf>,
    /// Audio d'histoire proposé pour la liste du mode nuit.
    night_candidate: Option<PathBuf>,
}

impl Stories {
    pub fn new(ctx: &mut Ctx, night_mode: bool, session: Option<SessionState>) -> Stories {
        ctx.show_res("loadingStories.png");
        let list = scan_stories(&ctx.paths.stories());
        let has_save = scan_saves(&ctx.paths.story_saves(), &list);
        let mut stories = Stories {
            night_mode,
            list,
            has_save,
            index: 0,
            tiles: ctx.params.story_display_tiles,
            player: None,
            autoplay: false,
            ok_action: true,
            screen_enabled: true,
            show_timeline: false,
            timeline_second: -1,
            timeline_drawn: -1,
            timeline_until: 0,
            hook: Hook::None,
            night_playing: false,
            night_index: 0,
            night_playlist: Vec::new(),
            night_candidate: None,
        };
        stories.restore(ctx, session);
        stories
    }

    fn restore(&mut self, ctx: &mut Ctx, session: Option<SessionState>) {
        match session {
            Some(SessionState::NightModePlaying(n)) if !n.playlist.is_empty() => {
                self.night_mode = true;
                self.night_playlist = n.playlist.iter().map(PathBuf::from).take(NIGHT_PLAYLIST_MAX).collect();
                self.night_index = n.index.min(self.night_playlist.len() - 1);
                return self.night_resume(ctx, n.time);
            }
            Some(SessionState::Stories(s)) | Some(SessionState::NightModeTitle(s)) => {
                if let Some(index) = self.position(&s.story_dir) {
                    self.index = index;
                    if !s.is_title_only() && !self.night_mode {
                        return self.open(ctx, Some((s.resume, s.time)));
                    }
                }
            }
            _ => {}
        }
        self.title(ctx);
    }

    fn position(&self, dir: &str) -> Option<usize> {
        self.list.binary_search_by(|s| s.as_str().cmp(dir)).ok()
    }

    fn story_dir(&self, ctx: &Ctx) -> PathBuf {
        ctx.paths.stories().join(&self.list[self.index])
    }

    fn in_story(&self) -> bool {
        self.player.is_some()
    }

    // --- Écran titre -------------------------------------------------------

    fn title(&mut self, ctx: &mut Ctx) {
        if self.list.is_empty() {
            ctx.show_res("noStory.png");
            return;
        }
        ctx.autosleep_default();
        self.autoplay = false;
        let dir = self.story_dir(ctx);
        ctx.audio.play(&dir.join("title.mp3"), 0.0);
        self.hook = Hook::None;
        self.screen_enabled = true;
        ctx.set_screen(true);
        self.draw_title(ctx);
    }

    fn draw_title(&mut self, ctx: &mut Ctx) {
        if self.tiles {
            self.draw_tiles(ctx);
        } else {
            ctx.black();
            let dir = self.story_dir(ctx);
            ctx.image(&dir.join("title.png"), 0, 0, 640);
            if !self.night_mode && self.has_save[self.index] {
                ctx.res("storytellerResumeStory.png", 15, 15, 48);
            }
            ctx.present();
        }
    }

    fn draw_tiles(&mut self, ctx: &mut Ctx) {
        let base = self.index / TILES_PER_PAGE * TILES_PER_PAGE;
        ctx.black();
        ctx.res("storiesTiles.png", 0, 0, 640);
        for pos in 0..TILES_PER_PAGE {
            let i = base + pos;
            if i >= self.list.len() {
                break;
            }
            let x = 33 + (pos % 3) as i32 * 201;
            let y = 28 + (pos / 3) as i32 * 148;
            if i == self.index {
                ctx.rect(x - 5, y - 5, 181, 138, Color::YELLOW);
            }
            ctx.rect(x, y, 171, 128, Color::BLACK);
            let path = ctx.paths.stories().join(&self.list[i]).join("title.png");
            ctx.image(&path, x, y, 171);
            if !self.night_mode && self.has_save[i] {
                ctx.res("storytellerResumeStory.png", x + 5, y + 5, 48);
            }
        }
        if self.night_mode {
            self.draw_night_counter(ctx);
        }
        let page = format!("{} / {}", self.index + 1, self.list.len());
        ctx.write(&page, TEXT_16, 606, 456, Align::Right);
        ctx.present();
    }

    fn change_title(&mut self, ctx: &mut Ctx, delta: isize) {
        if self.autoplay || self.in_story() || self.list.is_empty() {
            return;
        }
        self.index = (self.index as isize + delta).rem_euclid(self.list.len() as isize) as usize;
        self.title(ctx);
    }

    // --- Histoire ------------------------------------------------------------

    /// Ouvre l'histoire sélectionnée, éventuellement à une position sauvegardée.
    fn open(&mut self, ctx: &mut Ctx, resume: Option<(Resume, f64)>) {
        let dir = self.story_dir(ctx);
        let mut resume = resume;
        if resume.is_none() && !self.night_mode && self.has_save[self.index] {
            resume = self.take_story_save(ctx);
        }
        let story = match load_story(&dir.join("nodes.json")) {
            Ok(story) => story,
            Err(e) => {
                log!("histoire {} illisible : {e}", dir.display());
                return self.reset(ctx);
            }
        };
        let seed = ctx.now_ms() as u64 ^ (self.index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut player = Player::new(story, seed);
        let now = ctx.now();
        let (resume, time) = resume.map_or((None, 0.0), |(r, t)| (Some(r), t));
        let step = player.start(resume.as_ref(), now);
        self.player = Some(player);
        self.on_step(ctx, step, time);
    }

    fn take_story_save(&mut self, ctx: &Ctx) -> Option<(Resume, f64)> {
        let path = ctx.paths.story_saves().join(&self.list[self.index]);
        let state = save::read_session(&path);
        let _ = std::fs::remove_file(&path);
        self.has_save[self.index] = false;
        match state {
            Some(SessionState::Stories(s)) if !s.is_title_only() => Some((s.resume, s.time)),
            _ => None,
        }
    }

    fn on_step(&mut self, ctx: &mut Ctx, step: Step, time: f64) {
        match step {
            Step::Stage(_) => self.present_stage(ctx, time),
            Step::Reset => self.reset(ctx),
        }
    }

    /// Lecture de l'étape courante, comme `stories_readStage`.
    fn present_stage(&mut self, ctx: &mut Ctx, time: f64) {
        let Some(player) = self.player.as_ref() else { return };
        let Some(stage) = player.current_stage().copied() else { return self.reset(ctx) };
        let story = player.story();
        let dir = self.story_dir(ctx);
        let audio = stage.audio.map(|a| dir.join("audios").join(story.str(a)));
        let image = stage.image.map(|i| dir.join("images").join(story.str(i)));

        self.autoplay = stage.autoplay;
        self.ok_action = true;
        self.show_timeline = false;
        self.hook = Hook::None;

        match &audio {
            Some(path) => {
                ctx.audio.play(path, time);
                if self.autoplay {
                    self.hook = Hook::Autoplay;
                    self.ok_action = stage.ok_enabled;
                    ctx.autosleep.lock();
                } else {
                    ctx.autosleep_default();
                }
            }
            None => {
                ctx.audio.stop();
                ctx.autosleep_default();
            }
        }

        if let Some(image) = image {
            ctx.set_screen(true);
            self.screen_enabled = true;
            ctx.black();
            ctx.image(&image, 0, 0, 640);
            self.draw_inventory(ctx);
            if self.night_mode {
                self.night_candidate = None;
                self.draw_night_counter(ctx);
            }
            ctx.present();
        } else {
            self.screen_enabled = false;
            if self.night_mode && self.autoplay && !self.ok_action {
                self.night_candidate = audio;
                ctx.set_screen(true);
                ctx.black();
                ctx.res("storytellerNightMode.png", 0, 0, 640);
                self.draw_night_counter(ctx);
                ctx.present();
            } else {
                self.show_timeline = !ctx.params.story_disable_timeline;
                ctx.black_screen();
                if !ctx.overlay_active() {
                    ctx.set_screen(false);
                }
                self.night_candidate = None;
            }
        }
    }

    fn draw_inventory(&mut self, ctx: &mut Ctx) {
        let Some(player) = self.player.as_ref() else { return };
        let story = player.story();
        let items = story.inventory();
        let counts = player.inventory();
        let visible: Vec<usize> = (0..items.len())
            .filter(|&i| counts.get(i).copied().unwrap_or(0) > 0 && items[i].display < 2)
            .take(8)
            .collect();
        if visible.is_empty() {
            return;
        }
        ctx.res("storytellerInventoryBg.png", 0, 384, 640);
        let width = 80;
        let x0 = 320 - visible.len() as i32 * width / 2;
        let images = self.story_dir(ctx).join("images");
        for (slot, &i) in visible.iter().enumerate() {
            let item = items[i];
            let count = counts[i];
            let x = x0 + slot as i32 * width;
            let y = 400;
            if let Some(image) = item.image {
                ctx.image(&images.join(story.str(image)), x, y, width as usize);
            }
            match item.display {
                0 if item.max > 1 => {
                    let style = Style::new(Weight::Bold, 24, Color::WHITE);
                    ctx.write(&count.to_string(), style, x + width - 2, y + width - 27, Align::Right);
                }
                1 if item.max > 0 => {
                    ctx.rect(x + 4, y + 73, 72, 3, Color(75, 75, 75));
                    ctx.rect(x + 4, y + 73, count * 72 / item.max, 3, Color(230, 230, 230));
                    ctx.res("storytellerInventoryBar.png", x + 2, y + 71, 76);
                }
                _ => {}
            }
        }
    }

    fn draw_timeline(&mut self, ctx: &mut Ctx, force: bool) {
        let position = ctx.audio.position();
        let second = position as i64;
        if !force && second == self.timeline_drawn {
            return;
        }
        self.timeline_drawn = second;
        let duration = ctx.audio.duration();
        ctx.black();
        if let Some(d) = duration.filter(|d| *d > 0.0) {
            ctx.rect(80, 217, ((position / d).min(1.0) * 479.0) as i32, 19, Color::YELLOW);
        }
        ctx.res("storytellerStoryPlayer.png", 61, 203, 518);
        let icon = if ctx.audio.is_paused() { "storytellerPause.png" } else { "storytellerPlay.png" };
        ctx.res(icon, 308, 245, 24);
        if self.ok_action {
            ctx.res("storytellerSkip.png", 270, 300, 101);
        }
        let style = Style::new(Weight::Regular, 20, Color::WHITE);
        ctx.write(&format_time(Some(position)), style, 85, 244, Align::Left);
        ctx.write(&format_time(duration), style, 554, 244, Align::Right);
        if self.night_playing {
            let count = format!("{} / {}", self.night_index + 1, self.night_playlist.len());
            ctx.write(&count, style, 298, 244, Align::Right);
        }
        self.draw_inventory(ctx);
        ctx.present();
    }

    fn show_timeline(&mut self, ctx: &mut Ctx) {
        if self.show_timeline {
            self.timeline_until = ctx.now() + TIMELINE_SECS;
            ctx.set_screen(true);
            self.draw_timeline(ctx, true);
        }
    }

    fn screen_update(&mut self, ctx: &mut Ctx) {
        if !ctx.screen_on() || !self.show_timeline {
            return;
        }
        let now = ctx.now();
        if now != self.timeline_second {
            self.timeline_second = now;
            if !ctx.overlay_active() && now > self.timeline_until {
                ctx.black_screen();
                ctx.set_screen(false);
                return;
            }
        }
        self.draw_timeline(ctx, false);
    }

    /// Retour à l'écran titre, comme `stories_reset`.
    fn reset(&mut self, ctx: &mut Ctx) {
        self.hook = Hook::None;
        self.autoplay = false;
        self.ok_action = true;
        self.show_timeline = false;
        self.timeline_second = -1;
        self.timeline_until = 0;
        self.player = None;
        self.title(ctx);
    }

    fn transition(&mut self, ctx: &mut Ctx, home: bool) {
        let now = ctx.now();
        let Some(player) = self.player.as_mut() else { return };
        self.hook = Hook::None;
        let step = if home { player.home(now) } else { player.ok(now) };
        self.on_step(ctx, step, 0.0);
    }

    fn rewind(&mut self, ctx: &mut Ctx, delta: f64) {
        let target = ctx.audio.position() + delta;
        let duration = ctx.audio.duration().filter(|d| *d > 0.0);
        match duration {
            Some(d) if target >= d => {
                if self.hook != Hook::None {
                    return self.fire_hook(ctx);
                }
                ctx.audio.seek(d - 1.0);
            }
            _ => ctx.audio.seek(target.max(0.0)),
        }
    }

    fn fire_hook(&mut self, ctx: &mut Ctx) {
        match self.hook {
            Hook::Autoplay => {
                self.ok_action = true;
                self.ok(ctx);
            }
            Hook::NightNext => self.night_next(ctx),
            Hook::None => {}
        }
    }

    // --- Mode nuit -------------------------------------------------------------

    fn draw_night_counter(&mut self, ctx: &mut Ctx) {
        ctx.res("storytellerNightModeCounter.png", 33, 2, 64);
        let style = Style::new(Weight::Bold, 20, Color::PURPLE);
        ctx.write(&self.night_playlist.len().to_string(), style, 74, 1, Align::Center);
    }

    /// Ajoute l'audio proposé ; vrai si la liste est pleine.
    fn night_add(&mut self) -> bool {
        if self.night_playlist.len() < NIGHT_PLAYLIST_MAX {
            if let Some(path) = self.night_candidate.clone() {
                self.night_playlist.push(path);
            }
        }
        self.night_playlist.len() >= NIGHT_PLAYLIST_MAX
    }

    fn night_start(&mut self, ctx: &mut Ctx) {
        self.autoplay = false;
        self.night_index = 0;
        self.night_resume(ctx, 0.0);
    }

    fn night_resume(&mut self, ctx: &mut Ctx, time: f64) {
        ctx.autosleep.lock();
        ctx.black_screen();
        ctx.set_screen(false);
        self.screen_enabled = false;
        self.night_playing = true;
        self.show_timeline = !ctx.params.story_disable_timeline;
        self.night_candidate = None;
        self.night_play(ctx, time);
    }

    fn night_play(&mut self, ctx: &mut Ctx, time: f64) {
        self.hook = Hook::NightNext;
        if let Some(path) = self.night_playlist.get(self.night_index) {
            ctx.audio.play(path, time);
        }
    }

    fn night_next(&mut self, ctx: &mut Ctx) {
        self.night_index += 1;
        if self.night_index < self.night_playlist.len() {
            self.night_play(ctx, 0.0);
        } else {
            self.night_playing = false;
            self.show_timeline = false;
            self.hook = Hook::None;
            ctx.autosleep_unlock(5, 5);
        }
    }

    fn night_clear(&mut self) {
        self.night_playing = false;
        self.show_timeline = false;
        self.night_index = 0;
        self.night_playlist.clear();
        self.night_candidate = None;
    }

    // --- Commandes -------------------------------------------------------------

    /// Réouverture depuis le sélecteur : retour à l'écran titre.
    pub fn reopen(&mut self, ctx: &mut Ctx) {
        self.reset(ctx);
    }

    pub fn update(&mut self, ctx: &mut Ctx) {
        if self.hook != Hook::None && ctx.audio.is_finished() {
            self.fire_hook(ctx);
        } else {
            self.screen_update(ctx);
        }
    }

    pub fn ok(&mut self, ctx: &mut Ctx) {
        if self.list.is_empty() {
            return;
        }
        if self.night_playing {
            return self.show_timeline(ctx);
        }
        if !self.in_story() {
            return self.open(ctx, None);
        }
        if self.night_candidate.is_some() {
            if self.night_add() {
                self.night_start(ctx);
            } else {
                self.transition(ctx, true);
            }
        } else if self.ok_action {
            self.transition(ctx, false);
        } else {
            self.show_timeline(ctx);
        }
    }

    /// Vrai si l'application doit rendre la main au sélecteur.
    pub fn home(&mut self, ctx: &mut Ctx) -> bool {
        if self.list.is_empty() {
            self.night_clear();
            return true;
        }
        if !self.in_story() || self.night_playing {
            self.hook = Hook::None;
            ctx.audio.stop();
            self.night_clear();
            return true;
        }
        self.transition(ctx, true);
        false
    }

    pub fn menu(&mut self, ctx: &mut Ctx) {
        if self.list.is_empty() || self.night_playing {
            return;
        }
        if !self.in_story() {
            self.tiles = !self.tiles;
            self.draw_title(ctx);
        } else {
            self.save_current_story(ctx);
            self.reset(ctx);
        }
    }

    pub fn next(&mut self, ctx: &mut Ctx) {
        self.horizontal(ctx, 1);
    }

    pub fn previous(&mut self, ctx: &mut Ctx) {
        self.horizontal(ctx, -1);
    }

    fn horizontal(&mut self, ctx: &mut Ctx, direction: i32) {
        if self.list.is_empty() {
            return;
        }
        if self.autoplay || self.night_playing {
            self.rewind(ctx, 10.0 * direction as f64);
            self.show_timeline(ctx);
        } else if !self.in_story() {
            self.change_title(ctx, direction as isize);
        } else if let Some(player) = self.player.as_mut() {
            let now = ctx.now();
            let step = player.select(direction, now);
            self.on_step(ctx, step, 0.0);
        }
    }

    pub fn up(&mut self, ctx: &mut Ctx) {
        self.vertical(ctx, -3);
    }

    pub fn down(&mut self, ctx: &mut Ctx) {
        self.vertical(ctx, 3);
    }

    fn vertical(&mut self, ctx: &mut Ctx, delta: isize) {
        if self.list.is_empty() {
            return;
        }
        if self.autoplay || self.night_playing {
            self.show_timeline(ctx);
        } else if !self.in_story() && self.tiles {
            self.change_title(ctx, delta);
        }
    }

    pub fn pause(&mut self, ctx: &mut Ctx) {
        if self.list.is_empty() {
            return;
        }
        if self.night_candidate.is_some() {
            self.night_add();
            return self.night_start(ctx);
        }
        if ctx.audio.is_playing() {
            if ctx.audio.is_paused() {
                ctx.autosleep.lock();
                ctx.audio.resume();
            } else {
                ctx.autosleep_default();
                ctx.audio.pause();
            }
            self.show_timeline(ctx);
        }
    }

    /// Redessine après une surcouche (verrou, volume…), comme `stories_forceRefreshScreen`.
    pub fn refresh(&mut self, ctx: &mut Ctx) {
        let on = ctx.overlay_active() || self.screen_enabled;
        ctx.set_screen(on);
        ctx.present();
    }

    fn save_current_story(&mut self, ctx: &mut Ctx) {
        if self.night_mode {
            return;
        }
        if let Some(state) = self.story_state(ctx) {
            let path = ctx.paths.story_saves().join(&self.list[self.index]);
            match save::write_session(&path, &SessionState::Stories(state)) {
                Ok(()) => self.has_save[self.index] = true,
                Err(e) => log!("sauvegarde {} : {e}", path.display()),
            }
        }
    }

    fn story_state(&mut self, ctx: &Ctx) -> Option<StorySave> {
        let story_dir = self.list.get(self.index)?.clone();
        let now = ctx.now();
        let time = if self.autoplay { ctx.audio.position() } else { 0.0 };
        let resume = self.player.as_mut().map(|p| p.snapshot(now)).unwrap_or_default();
        Some(StorySave { story_dir, time, resume })
    }

    /// État à enregistrer à l'extinction, comme `stories_save`.
    pub fn session(&mut self, ctx: &mut Ctx) -> Option<SessionState> {
        if self.list.is_empty() {
            return None;
        }
        if ctx.audio.is_playing() && !ctx.audio.is_paused() && self.night_candidate.is_none() {
            ctx.audio.pause();
        }
        if self.night_playing {
            return Some(SessionState::NightModePlaying(NightSave {
                playlist: self.night_playlist.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
                index: self.night_index,
                time: ctx.audio.position(),
            }));
        }
        let state = self.story_state(ctx)?;
        Some(if self.night_mode { SessionState::NightModeTitle(state) } else { SessionState::Stories(state) })
    }
}

/// Dossiers d'histoires triés comme Telmi (ordre des octets), sans dossiers cachés.
fn scan_stories(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        log!("dossier {} illisible", root.display());
        return Vec::new();
    };
    let mut list: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .filter(|name| root.join(name).join("nodes.json").is_file())
        .collect();
    list.sort();
    list
}

fn scan_saves(root: &Path, list: &[String]) -> Vec<bool> {
    let mut has_save = vec![false; list.len()];
    for name in std::fs::read_dir(root).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()) {
        if let Ok(i) = list.binary_search(&name) {
            has_save[i] = true;
        }
    }
    has_save
}
