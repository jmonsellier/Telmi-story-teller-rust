//! Lecteur de musique, portage de `music_player.h`.
//!
//! Fichiers `/Music/Artiste_Album_Piste_Titre.mp3` avec pochette
//! `Artiste_Album_Piste_Titre.png`. Contrairement à Telmi OS, un nom sans
//! les trois `_` attendus s'affiche tel quel au lieu de faire planter.

use std::path::PathBuf;

use telmi_story::save::{MusicSave, SessionState};

use crate::gfx::Color;
use crate::text::{Align, Style, Weight};
use crate::ui::{format_time, Ctx, TEXT_16};

/// Extinction de l'écran après 10 s sans appui.
const SCREEN_IDLE_SECS: i64 = 10;
const ALBUMS_PER_PAGE: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Repeat {
    All,
    Shuffle,
    Album,
    Title,
}

impl Repeat {
    fn next(self) -> Repeat {
        match self {
            Repeat::All => Repeat::Shuffle,
            Repeat::Shuffle => Repeat::Album,
            Repeat::Album => Repeat::Title,
            Repeat::Title => Repeat::All,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Repeat::All => "musicPlayerRepeatMode0.png",
            Repeat::Shuffle => "musicPlayerRepeatMode1.png",
            Repeat::Album => "musicPlayerRepeatMode2.png",
            Repeat::Title => "musicPlayerRepeatMode3.png",
        }
    }
}

/// Champs affichés d'une piste.
struct Track {
    file: String,
    title: String,
    artist: String,
}

impl Track {
    fn parse(file: String) -> Track {
        let stem = file.strip_suffix(".mp3").unwrap_or(&file);
        let parts: Vec<&str> = stem.splitn(4, '_').collect();
        let (title, artist) = match parts.as_slice() {
            [artist, album, track, title] => (format!("{track}. {title}"), format!("{artist} - {album}")),
            _ => (stem.to_owned(), String::new()),
        };
        Track { file, title, artist }
    }

    fn album_key(&self) -> &str {
        let mut cut = self.file.match_indices('_').map(|(i, _)| i);
        match (cut.next(), cut.next()) {
            (Some(_), Some(second)) => &self.file[..second],
            _ => &self.file,
        }
    }

    fn cover(&self) -> String {
        format!("{}.png", self.file.strip_suffix(".mp3").unwrap_or(&self.file))
    }
}

pub struct Music {
    tracks: Vec<Track>,
    /// Index de la première piste de chaque album.
    albums: Vec<usize>,
    index: usize,
    position: f64,
    album_view: bool,
    album_index: usize,
    repeat: Repeat,
    drawn_second: i64,
    screen_second: i64,
    last_activity: i64,
    rng: u64,
}

impl Music {
    pub fn new(ctx: &mut Ctx, session: Option<SessionState>) -> Music {
        ctx.show_res("loadingMusic.png");
        let mut files: Vec<String> = std::fs::read_dir(ctx.paths.music())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.ends_with(".mp3") && !name.starts_with('.'))
            .collect();
        files.sort();
        let tracks: Vec<Track> = files.into_iter().map(Track::parse).collect();
        let albums = tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| *i == 0 || tracks[i - 1].album_key() != t.album_key())
            .map(|(i, _)| i)
            .collect();
        let (index, position) = match session {
            Some(SessionState::Music(MusicSave { index, position })) => (index, position),
            _ => (0, 0.0),
        };
        let mut music = Music {
            tracks,
            albums,
            index,
            position,
            album_view: false,
            album_index: 0,
            repeat: Repeat::All,
            drawn_second: -1,
            screen_second: -1,
            last_activity: 0,
            rng: ctx.now_ms() as u64 | 1,
        };
        music.lock_autosleep(ctx);
        music.activate(ctx);
        music.load(ctx);
        music
    }

    fn lock_autosleep(&self, ctx: &mut Ctx) {
        let secs = ctx.params.music_inactivity;
        ctx.autosleep_unlock(secs, secs);
    }

    fn activate(&mut self, ctx: &mut Ctx) {
        self.last_activity = ctx.now();
        ctx.set_screen(true);
    }

    fn path(&self, ctx: &Ctx, file: &str) -> PathBuf {
        ctx.paths.music().join(file)
    }

    fn load(&mut self, ctx: &mut Ctx) {
        if self.tracks.is_empty() {
            ctx.show_res("noMusic.png");
            return;
        }
        self.index %= self.tracks.len();
        let paused = ctx.audio.is_playing() && ctx.audio.is_paused();
        let path = self.path(ctx, &self.tracks[self.index].file);
        ctx.audio.play(&path, self.position);
        if paused {
            ctx.audio.pause();
        }
        if ctx.screen_on() && !self.album_view {
            self.draw_player(ctx, true);
        }
    }

    fn current_album(&self) -> usize {
        self.albums.iter().rposition(|&start| start <= self.index).unwrap_or(0)
    }

    fn album_range(&self, album: usize) -> std::ops::Range<usize> {
        let start = self.albums[album];
        let end = self.albums.get(album + 1).copied().unwrap_or(self.tracks.len());
        start..end
    }

    fn change_song(&mut self, ctx: &mut Ctx, direction: isize) {
        if self.tracks.is_empty() {
            return;
        }
        self.position = 0.0;
        let count = self.tracks.len() as isize;
        self.index = match self.repeat {
            Repeat::All => (self.index as isize + direction).rem_euclid(count) as usize,
            Repeat::Shuffle => {
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 7;
                self.rng ^= self.rng << 17;
                (self.rng % count as u64) as usize
            }
            Repeat::Album => {
                let range = self.album_range(self.current_album());
                let len = range.len() as isize;
                range.start + (self.index as isize - range.start as isize + direction).rem_euclid(len) as usize
            }
            // Répéter le titre : la fin de piste rejoue, les boutons changent de piste.
            Repeat::Title if direction == 0 => self.index,
            Repeat::Title => (self.index as isize + direction).rem_euclid(count) as usize,
        };
        self.load(ctx);
    }

    // --- Dessin --------------------------------------------------------------

    fn draw_player(&mut self, ctx: &mut Ctx, force: bool) {
        let position = ctx.audio.position();
        if !force && position as i64 == self.drawn_second {
            return;
        }
        self.drawn_second = position as i64;
        let duration = ctx.audio.duration();
        let track = &self.tracks[self.index];
        ctx.black();
        if let Some(d) = duration.filter(|d| *d > 0.0) {
            ctx.rect(185, 258, ((position / d).min(1.0) * 422.0) as i32, 12, Color::YELLOW);
        }
        ctx.res("musicPlayer.png", 0, 0, 640);
        let cover = self.path(ctx, &track.cover());
        ctx.image(&cover, 24, 176, 128);
        let (title, artist) = (track.title.clone(), track.artist.clone());
        ctx.write(&title, Style::new(Weight::Bold, 24, Color::WHITE), 185, 190, Align::Left);
        ctx.write(&artist, Style::new(Weight::Regular, 20, Color::WHITE), 185, 222, Align::Left);
        let small = Style::new(Weight::Regular, 18, Color::WHITE);
        ctx.write(&format_time(Some(position)), small, 185, 275, Align::Left);
        ctx.write(&format_time(duration), small, 605, 275, Align::Right);
        if !ctx.params.music_disable_repeat_modes {
            ctx.res(self.repeat.icon(), 332, 274, 128);
        }
        for (offset, top) in [(-2, 0), (-1, 83), (1, 314), (2, 397)] {
            self.draw_side(ctx, offset, top);
        }
        ctx.present();
    }

    fn draw_side(&self, ctx: &mut Ctx, offset: isize, top: i32) {
        let count = self.tracks.len() as isize;
        if count < 2 {
            return;
        }
        let track = &self.tracks[(self.index as isize + offset).rem_euclid(count) as usize];
        let cover = self.path(ctx, &track.cover());
        ctx.image(&cover, 55, top + 9, 64);
        ctx.write(&track.title, Style::new(Weight::Bold, 20, Color::WHITE_60), 150, top + 17, Align::Left);
        ctx.write(&track.artist, TEXT_16, 150, top + 43, Align::Left);
    }

    fn draw_albums(&mut self, ctx: &mut Ctx) {
        let count = self.albums.len();
        if count == 0 {
            return;
        }
        self.album_index %= count;
        let base = self.album_index / ALBUMS_PER_PAGE * ALBUMS_PER_PAGE;
        ctx.black();
        ctx.res("musicPlayerAlbums.png", 0, 0, 640);
        for pos in 0..ALBUMS_PER_PAGE.min(count - base) {
            let album = base + pos;
            let x = 51 + (pos % 3) as i32 * 205;
            let y = 28 + (pos / 3) as i32 * 148;
            if album == self.album_index {
                ctx.rect(x - 5, y - 5, 138, 138, Color::YELLOW);
            }
            let cover = self.path(ctx, &self.tracks[self.albums[album]].cover());
            ctx.image(&cover, x, y, 128);
        }
        ctx.write(&format!("{} / {}", self.album_index + 1, count), TEXT_16, 590, 456, Align::Right);
        ctx.present();
    }

    fn set_view(&mut self, ctx: &mut Ctx, albums: bool) {
        self.album_view = albums;
        if albums {
            self.album_index = self.current_album();
            self.draw_albums(ctx);
        } else {
            self.screen_update(ctx, true);
        }
    }

    fn change_album(&mut self, ctx: &mut Ctx, delta: isize) {
        let count = self.albums.len() as isize;
        if count > 0 {
            self.album_index = (self.album_index as isize + delta).rem_euclid(count) as usize;
            self.draw_albums(ctx);
        }
    }

    fn screen_update(&mut self, ctx: &mut Ctx, force: bool) {
        if !ctx.screen_on() || self.tracks.is_empty() {
            return;
        }
        let now = ctx.now();
        if self.screen_second != now {
            self.screen_second = now;
            if now - self.last_activity > SCREEN_IDLE_SECS && !ctx.overlay_active() {
                ctx.black_screen();
                ctx.set_screen(false);
                return;
            }
        }
        if !self.album_view {
            self.draw_player(ctx, force);
        }
    }

    // --- Commandes -------------------------------------------------------------

    pub fn update(&mut self, ctx: &mut Ctx) {
        if !self.tracks.is_empty() && ctx.audio.is_finished() {
            self.change_song(ctx, if self.repeat == Repeat::Title { 0 } else { 1 });
        } else {
            self.screen_update(ctx, false);
        }
    }

    pub fn up(&mut self, ctx: &mut Ctx) {
        self.vertical(ctx, -1);
    }

    pub fn down(&mut self, ctx: &mut Ctx) {
        self.vertical(ctx, 1);
    }

    fn vertical(&mut self, ctx: &mut Ctx, direction: isize) {
        if self.tracks.is_empty() {
            return;
        }
        self.activate(ctx);
        if self.album_view {
            self.change_album(ctx, 3 * direction);
        } else {
            self.change_song(ctx, direction);
        }
    }

    pub fn next(&mut self, ctx: &mut Ctx) {
        self.horizontal(ctx, 1);
    }

    pub fn previous(&mut self, ctx: &mut Ctx) {
        self.horizontal(ctx, -1);
    }

    fn horizontal(&mut self, ctx: &mut Ctx, direction: isize) {
        if self.tracks.is_empty() {
            return;
        }
        self.activate(ctx);
        if self.album_view {
            return self.change_album(ctx, direction);
        }
        let target = ctx.audio.position() + 10.0 * direction as f64;
        match ctx.audio.duration() {
            Some(d) if target >= d => return self.change_song(ctx, 1),
            _ => ctx.audio.seek(target.max(0.0)),
        }
        self.draw_player(ctx, true);
    }

    pub fn ok(&mut self, ctx: &mut Ctx) {
        if self.tracks.is_empty() {
            return;
        }
        if !ctx.screen_on() {
            return self.activate(ctx);
        }
        if self.album_view {
            self.album_view = false;
            self.position = 0.0;
            self.index = self.albums[self.album_index];
            self.load(ctx);
        } else if !ctx.params.music_disable_repeat_modes {
            self.repeat = self.repeat.next();
            self.draw_player(ctx, true);
        }
    }

    pub fn pause(&mut self, ctx: &mut Ctx) {
        if self.tracks.is_empty() {
            return;
        }
        self.activate(ctx);
        if ctx.audio.is_playing() {
            if ctx.audio.is_paused() {
                ctx.audio.resume();
                self.lock_autosleep(ctx);
            } else {
                ctx.audio.pause();
                self.position = ctx.audio.position();
                ctx.autosleep_default();
            }
        }
    }

    /// Vrai si l'application doit rendre la main au sélecteur.
    pub fn home(&mut self, ctx: &mut Ctx) -> bool {
        if self.tracks.is_empty() {
            return true;
        }
        if self.album_view {
            self.activate(ctx);
            self.set_view(ctx, false);
            return false;
        }
        if ctx.audio.is_playing() && !ctx.audio.is_paused() {
            self.position = ctx.audio.position();
        }
        ctx.audio.stop();
        true
    }

    pub fn menu(&mut self, ctx: &mut Ctx) {
        if self.tracks.is_empty() {
            return;
        }
        self.activate(ctx);
        let albums = !self.album_view;
        self.set_view(ctx, albums);
    }

    /// Réouverture depuis le sélecteur : reprise de la piste en cours.
    pub fn reopen(&mut self, ctx: &mut Ctx) {
        self.lock_autosleep(ctx);
        self.activate(ctx);
        self.load(ctx);
    }

    pub fn refresh(&mut self, ctx: &mut Ctx) {
        if !ctx.screen_on() && ctx.overlay_active() {
            ctx.set_screen(true);
        }
        let albums = self.album_view;
        self.set_view(ctx, albums);
    }

    pub fn session(&mut self, ctx: &mut Ctx) -> Option<SessionState> {
        if self.tracks.is_empty() {
            return None;
        }
        if ctx.audio.is_playing() && !ctx.audio.is_paused() {
            ctx.audio.pause();
        }
        Some(SessionState::Music(MusicSave { index: self.index, position: ctx.audio.position() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_track_names() {
        let t = Track::parse("Renaud_Mistral gagnant_01_Mistral_gagnant.mp3".into());
        assert_eq!(t.title, "01. Mistral_gagnant");
        assert_eq!(t.artist, "Renaud - Mistral gagnant");
        assert_eq!(t.album_key(), "Renaud_Mistral gagnant");
        assert_eq!(t.cover(), "Renaud_Mistral gagnant_01_Mistral_gagnant.png");
        let odd = Track::parse("sans_tirets.mp3".into());
        assert_eq!(odd.title, "sans_tirets");
        assert_eq!(odd.album_key(), "sans_tirets.mp3");
    }
}
