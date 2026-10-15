//! Contexte partagé par les applications : ressources, dessin, surcouches
//! (batterie, verrou, barres de volume et de luminosité) et mise en veille.
//!
//! Reprend `sdl_helper.h`, `app_lock.h`, `app_volume.h`, `app_brightness.h`,
//! `app_battery.h` et `app_autosleep.h` de Telmi OS.

use std::path::Path;
use std::time::Instant;

use crate::audio::Audio;
use crate::config::{Parameters, Paths};
use crate::gfx::{Canvas, Color, ImageCache};
use crate::hw::{Hardware, BRIGHTNESS_MAX, VOLUME_MAX};
use crate::sdl::Sdl;
use crate::text::{Align, Style, Text, Weight};

/// Budget du cache d'images : une dizaine d'images plein écran.
const IMAGE_CACHE_BYTES: usize = 12 * 1024 * 1024;
/// Appui long sur Menu pour verrouiller ou déverrouiller.
const LOCK_HOLD_MS: i64 = 1200;
/// Durée d'affichage de l'icône de verrou après un changement.
const LOCK_NOTICE_MS: i64 = 2500;
/// Durée d'affichage des barres de volume et de luminosité.
const BAR_NOTICE_MS: i64 = 2000;
const BATTERY_REFRESH_MS: i64 = 10_000;

pub const TEXT_16: Style = Style::new(Weight::Regular, 16, Color::WHITE_60);

/// Horloge monotone, insensible aux changements d'heure du système.
pub struct Clock(Instant);

impl Clock {
    pub fn ms(&self) -> i64 {
        self.0.elapsed().as_millis() as i64
    }

    pub fn secs(&self) -> i64 {
        self.ms() / 1000
    }
}

/// Verrouillage de l'écran par appui long sur Menu.
#[derive(Default)]
pub struct Lock {
    locked: bool,
    recently_unlocked: bool,
    hold_since: Option<i64>,
    changed_at: Option<i64>,
}

impl Lock {
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn recently_changed(&self) -> bool {
        self.changed_at.is_some()
    }

    pub fn is_unlocking(&self) -> bool {
        self.locked && self.hold_since.is_some()
    }

    /// Début de l'appui sur Menu. Renvoie vrai s'il faut rafraîchir l'écran.
    pub fn start(&mut self, now: i64) -> bool {
        self.hold_since = Some(now);
        self.locked
    }

    pub fn stop(&mut self) -> bool {
        self.hold_since = None;
        self.locked
    }

    /// À appeler à chaque tour de boucle ; vrai si l'affichage doit changer.
    pub fn check(&mut self, now: i64) -> bool {
        if self.changed_at.is_some_and(|t| now - t > LOCK_NOTICE_MS) {
            self.changed_at = None;
            self.recently_unlocked = false;
            return true;
        }
        if self.hold_since.is_some_and(|t| now - t > LOCK_HOLD_MS) {
            self.hold_since = None;
            self.locked = !self.locked;
            self.recently_unlocked = !self.locked;
            self.changed_at = Some(now);
            return true;
        }
        false
    }
}

/// Extinction automatique après inactivité (Telmi éteint alors la console).
pub struct AutoSleep {
    locked: bool,
    deadline: i64,
    screen_on_secs: i64,
    screen_off_secs: i64,
}

impl AutoSleep {
    pub fn is_sleeping_time(&self, now: i64) -> bool {
        !self.locked && now > self.deadline
    }

    /// Empêche l'extinction (lecture en cours).
    pub fn lock(&mut self) {
        self.locked = true;
    }

    pub fn unlock(&mut self, screen_on_secs: i64, screen_off_secs: i64, screen_on: bool, now: i64) {
        self.screen_on_secs = screen_on_secs;
        self.screen_off_secs = screen_off_secs;
        self.keep_awake(screen_on, now);
        self.locked = false;
    }

    pub fn keep_awake(&mut self, screen_on: bool, now: i64) {
        self.deadline = now + if screen_on { self.screen_on_secs } else { self.screen_off_secs };
    }
}

pub struct Ctx {
    /// Premier champ, donc détruit en premier : la sortie audio SDL est fermée
    /// avant que l'état partagé avec son rappel ne soit libéré.
    sdl: Sdl,
    pub paths: Paths,
    pub params: Parameters,
    pub hw: Hardware,
    pub audio: Audio,
    pub images: ImageCache,
    pub text: Text,
    /// Surface de l'application, sans les surcouches.
    pub canvas: Canvas,
    frame: Canvas,
    pub clock: Clock,
    pub lock: Lock,
    pub autosleep: AutoSleep,
    volume_until: Option<i64>,
    brightness_until: Option<i64>,
    battery: (i32, i64),
    #[cfg(not(target_arch = "arm"))]
    snapshots: Option<crate::sim::Snapshots>,
}

impl Ctx {
    pub fn new(paths: Paths, params: Parameters, hw: Hardware, audio: Audio, sdl: Sdl) -> Ctx {
        let clock = Clock(Instant::now());
        let autosleep = AutoSleep {
            locked: false,
            deadline: params.screen_on_inactivity,
            screen_on_secs: params.screen_on_inactivity,
            screen_off_secs: params.screen_off_inactivity,
        };
        Ctx {
            text: Text::load(&paths.res()),
            paths,
            params,
            hw,
            audio,
            images: ImageCache::new(IMAGE_CACHE_BYTES),
            canvas: Canvas::default(),
            frame: Canvas::default(),
            sdl,
            clock,
            lock: Lock::default(),
            autosleep,
            volume_until: None,
            brightness_until: None,
            battery: (100, i64::MIN),
            #[cfg(not(target_arch = "arm"))]
            snapshots: crate::sim::Snapshots::from_env(),
        }
    }

    /// Libère SDL et l'audio (dans cet ordre) et rend l'accès au matériel.
    pub fn into_hardware(self) -> Hardware {
        let Ctx { sdl, audio, hw, .. } = self;
        drop(sdl);
        drop(audio);
        hw
    }

    pub fn sdl(&mut self) -> &mut Sdl {
        &mut self.sdl
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.ms()
    }

    pub fn now(&self) -> i64 {
        self.clock.secs()
    }

    // --- Dessin -----------------------------------------------------------

    pub fn black(&mut self) {
        self.canvas.clear();
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        self.canvas.fill_rect(x, y, w, h, color);
    }

    /// Image mise à la largeur `width` (0 = taille d'origine), comme `video_screenAddImage`.
    pub fn image(&mut self, path: &Path, x: i32, y: i32, width: usize) {
        if let Some(image) = self.images.get(path, width) {
            self.canvas.blit(&image, x, y);
        }
    }

    pub fn res(&mut self, name: &str, x: i32, y: i32, width: usize) {
        let path = self.paths.res_file(name);
        self.image(&path, x, y, width);
    }

    pub fn write(&mut self, text: &str, style: Style, x: i32, y: i32, align: Align) {
        self.text.draw(&mut self.canvas, text, style, x, y, align);
    }

    /// Écran noir avec une image de ressource centrée, comme `video_displayImage`.
    pub fn show_res(&mut self, name: &str) {
        self.canvas.clear();
        let path = self.paths.res_file(name);
        if let Some(image) = self.images.get(&path, 0) {
            let x = (crate::sdl::WIDTH as i32 - image.width as i32) / 2;
            let y = (crate::sdl::HEIGHT as i32 - image.height as i32) / 2;
            self.canvas.blit(&image, x, y);
        }
        self.present();
    }

    pub fn black_screen(&mut self) {
        self.canvas.clear();
        self.present();
    }

    /// Compose les surcouches et affiche, comme `video_applyToVideo`.
    pub fn present(&mut self) {
        self.frame.copy_from(&self.canvas);
        self.draw_battery();
        self.draw_lock();
        self.draw_bar();
        self.sdl.present(&self.frame.pixels);
        #[cfg(not(target_arch = "arm"))]
        if let Some(snapshots) = self.snapshots.as_mut() {
            snapshots.save(&self.frame.pixels);
        }
    }

    fn draw_battery(&mut self) {
        let now = self.now_ms();
        if now - self.battery.1 > BATTERY_REFRESH_MS {
            self.battery = (self.hw.battery_percent(), now);
        }
        let percent = self.battery.0;
        let (icon, color) = match percent {
            ..6 => ("storytellerBatteryEmpty.png", Color::RED),
            6..20 => ("storytellerBatteryLow.png", Color::ORANGE),
            20..60 => ("storytellerBatteryMedium.png", Color::WHITE_60),
            _ => ("storytellerBatteryFull.png", Color::WHITE_60),
        };
        if let Some(image) = self.images.get(&self.paths.res_file(icon), 76) {
            self.frame.blit(&image, 531, 2);
        }
        let style = Style { color, ..TEXT_16 };
        self.text.draw(&mut self.frame, &format!("{percent}%"), style, 555, 2, Align::Center);
    }

    fn draw_lock(&mut self) {
        if !self.lock.locked && !self.lock.recently_unlocked {
            return;
        }
        let name = if self.lock.locked { "storytellerLock.png" } else { "storytellerUnlock.png" };
        if let Some(image) = self.images.get(&self.paths.res_file(name), 0) {
            self.frame.blit(&image, 0, 0);
        }
    }

    fn draw_bar(&mut self) {
        let (value, limit, max, name) = if self.brightness_until.is_some() {
            (self.hw.brightness(), self.params.brightness_max(), BRIGHTNESS_MAX, "storytellerBrightnessBar.png")
        } else if self.volume_until.is_some() {
            (self.hw.volume(), self.params.volume_max(), VOLUME_MAX, "storytellerVolumeBar.png")
        } else {
            return;
        };
        let height = value * 350 / max;
        let height_max = limit * 350 / max;
        self.frame.fill_rect(19, 47, 26, 350, Color::BLACK);
        self.frame.fill_rect(19, 397 - height, 26, height, Color::YELLOW);
        if height_max < 350 {
            self.frame.fill_rect(19, 397 - height_max, 26, 2, Color::RED);
        }
        if let Some(image) = self.images.get(&self.paths.res_file(name), 0) {
            self.frame.blit(&image, 0, 0);
        }
    }

    // --- Écran, volume, luminosité -----------------------------------------

    pub fn screen_on(&self) -> bool {
        self.hw.screen_on()
    }

    pub fn set_screen(&mut self, on: bool) {
        let was_on = self.hw.screen_on();
        self.hw.set_screen(on);
        if on && !was_on {
            // Le contenu affiché pendant l'extinction a pu être perdu.
            self.present();
        }
    }

    /// Une surcouche demande que l'écran reste allumé.
    pub fn overlay_active(&self) -> bool {
        self.lock.recently_changed()
            || self.lock.is_unlocking()
            || self.volume_until.is_some()
            || self.brightness_until.is_some()
    }

    /// Monte (plafonné par Telmi Sync) ou baisse le volume, et affiche la barre.
    pub fn volume_step(&mut self, delta: i32) {
        let target = step_toward(self.hw.volume(), delta, self.params.volume_max());
        self.hw.set_volume(target);
        self.volume_until = Some(self.now_ms() + BAR_NOTICE_MS);
    }

    pub fn brightness_step(&mut self, delta: i32) {
        let target = step_toward(self.hw.brightness(), delta, self.params.brightness_max());
        self.hw.set_brightness(target);
        self.brightness_until = Some(self.now_ms() + BAR_NOTICE_MS);
    }

    /// Fait expirer les barres ; vrai si l'affichage doit être rafraîchi.
    pub fn check_bars(&mut self) -> bool {
        let now = self.now_ms();
        let mut changed = false;
        for until in [&mut self.volume_until, &mut self.brightness_until] {
            if until.is_some_and(|t| t <= now) {
                *until = None;
                changed = true;
            }
        }
        changed
    }

    // --- Mise en veille ----------------------------------------------------

    pub fn autosleep_default(&mut self) {
        let (on, off) = (self.params.screen_on_inactivity, self.params.screen_off_inactivity);
        self.autosleep_unlock(on, off);
    }

    pub fn autosleep_unlock(&mut self, screen_on_secs: i64, screen_off_secs: i64) {
        let (screen, now) = (self.screen_on(), self.now());
        self.autosleep.unlock(screen_on_secs, screen_off_secs, screen, now);
    }

    pub fn keep_awake(&mut self) {
        let (screen, now) = (self.screen_on(), self.now());
        self.autosleep.keep_awake(screen, now);
    }
}

/// Pas de réglage : la montée est plafonnée par `max`, la descente non (comme Telmi).
fn step_toward(current: i32, delta: i32, max: i32) -> i32 {
    if delta > 0 {
        (current + delta).min(max)
    } else {
        current + delta
    }
}

/// `m:ss`, ou `-:--` quand la durée est inconnue.
pub fn format_time(seconds: Option<f64>) -> String {
    match seconds {
        Some(s) if s >= 0.0 => {
            let s = s as i64;
            format!("{}:{:02}", s / 60, s % 60)
        }
        _ => "-:--".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_toggles_after_long_press() {
        let mut lock = Lock::default();
        lock.start(0);
        assert!(!lock.check(1000));
        assert!(lock.check(1300));
        assert!(lock.is_locked() && lock.recently_changed());
        assert!(lock.check(4000), "fin de l'affichage du verrou");
        lock.start(5000);
        assert!(lock.is_unlocking());
        lock.check(6300);
        assert!(!lock.is_locked());
    }

    #[test]
    fn autosleep_respects_lock_and_screen() {
        let mut sleep = AutoSleep { locked: false, deadline: 0, screen_on_secs: 120, screen_off_secs: 300 };
        sleep.unlock(120, 300, false, 0);
        assert!(!sleep.is_sleeping_time(299));
        assert!(sleep.is_sleeping_time(301));
        sleep.lock();
        assert!(!sleep.is_sleeping_time(10_000));
    }

    #[test]
    fn formats_time() {
        assert_eq!(format_time(Some(65.9)), "1:05");
        assert_eq!(format_time(None), "-:--");
    }
}
