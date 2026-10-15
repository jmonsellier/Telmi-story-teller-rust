//! `storyTeller` : conteuse d'histoires pour Miyoo Mini Plus.
//!
//! Remplaçant du binaire du même nom de Telmi OS, lancé par
//! `.tmp_update/runtime.sh`. Boucle principale portée de `storyTeller.c`, avec :
//! - une attente bloquante des boutons (Telmi tournait à vide, à 100 % du processeur) ;
//! - une reprise sur panique : l'application revient au sélecteur au lieu de figer ;
//! - un code de sortie non nul en cas d'échec répété, pour que `runtime.sh` relance.

mod app;
mod audio;
mod config;
mod gfx;
mod hw;
mod input;
mod log;
mod sdl;
#[cfg(not(target_arch = "arm"))]
mod sim;
mod text;
mod ui;

use std::panic::{self, AssertUnwindSafe};

use app::Shell;
use audio::Audio;
use config::{Parameters, Paths};
use hw::Hardware;
use input::{Button, Event, Input};
use sdl::Sdl;
use ui::Ctx;

/// Attente maximale des boutons entre deux mises à jour (frise, fin d'audio).
const TICK_MS: i32 = 50;
/// Anti-rebond entre deux actions.
const DEBOUNCE_MS: i64 = 50;
/// Appui long sur marche/arrêt pour éteindre.
const POWER_HOLD_MS: i64 = 1000;
/// Au-delà de ce nombre de paniques en une minute, on rend la main à `runtime.sh`.
const MAX_PANICS_PER_MINUTE: usize = 5;

fn main() {
    let paths = Paths::from_env();
    log::init(&paths.log_file());
    panic::set_hook(Box::new(|info| log!("PANIQUE : {info}")));
    log!("démarrage de storyTeller {}", env!("CARGO_PKG_VERSION"));
    let code = match run(paths) {
        Ok(()) => 0,
        Err(e) => {
            log!("arrêt sur erreur : {e}");
            1
        }
    };
    std::process::exit(code);
}

fn run(paths: Paths) -> Result<(), String> {
    let params = Parameters::load(&paths.parameters());
    let mut hw = Hardware::detect();
    hw.set_volume(params.volume_startup());
    hw.set_brightness(params.brightness_startup());

    let mut sdl = Sdl::new()?;
    let audio = Audio::new();
    let (callback, userdata) = audio.callback();
    if let Err(e) = sdl.open_audio(callback, userdata) {
        log!("pas de son : {e}");
    }
    #[cfg(target_arch = "arm")]
    let mut input = Input::open("/dev/input/event0");
    #[cfg(not(target_arch = "arm"))]
    let mut input = Input::desktop();
    let mut ctx = Ctx::new(paths, params, hw, audio, sdl);

    let mut shell = guarded(|| Shell::start(&mut ctx)).ok_or("démarrage impossible")?;
    let mut keys = Keys::default();
    let mut events = Vec::new();
    let mut panics: Vec<i64> = Vec::new();

    loop {
        match guarded(|| tick(&mut ctx, &mut shell, &mut input, &mut keys, &mut events)) {
            Some(true) => {}
            Some(false) => break,
            None => {
                let now = ctx.now();
                panics.retain(|t| now - t < 60);
                panics.push(now);
                if panics.len() > MAX_PANICS_PER_MINUTE {
                    return Err("trop de paniques".into());
                }
                // État incohérent : on repart du sélecteur, sans session.
                ctx.audio.stop();
                keys = Keys::default();
                shell = guarded(|| Shell::start(&mut ctx)).ok_or("redémarrage impossible")?;
            }
        }
    }

    shell.save(&mut ctx);
    ctx.set_screen(true);
    ctx.audio.stop();
    let paths = ctx.paths.clone();
    let hw = ctx.into_hardware();
    hw.request_shutdown(&paths);
    log!("extinction demandée");
    Ok(())
}

/// Exécute `f` en interceptant une panique (déjà journalisée par le hook).
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    panic::catch_unwind(AssertUnwindSafe(f)).ok()
}

#[derive(Default)]
struct Keys {
    menu_pressed: bool,
    menu_prevent_default: bool,
    power_since: Option<i64>,
    last_action: i64,
}

/// Un tour de boucle ; `false` pour éteindre.
fn tick(ctx: &mut Ctx, shell: &mut Shell, input: &mut Input, keys: &mut Keys, events: &mut Vec<Event>) -> bool {
    let now = ctx.now_ms();
    if ctx.autosleep.is_sleeping_time(ctx.now()) || keys.power_since.is_some_and(|t| now - t > POWER_HOLD_MS) {
        return false;
    }
    let mut refresh = ctx.lock.check(now);
    refresh |= ctx.check_bars();
    shell.update(ctx);

    events.clear();
    input.wait(ctx.sdl(), TICK_MS, events);
    for &event in events.iter() {
        let now = ctx.now_ms();
        match event {
            Event::Quit => return false,
            Event::Pressed(Button::Menu) => {
                keys.menu_pressed = true;
                refresh |= ctx.lock.start(now);
                if ctx.lock.is_locked() {
                    keys.menu_prevent_default = true;
                }
            }
            Event::Pressed(Button::Power) if !ctx.lock.is_locked() => keys.power_since = Some(now),
            Event::Pressed(_) => {}
            Event::Released(button) => {
                if ctx.lock.is_locked() {
                    if button == Button::Menu {
                        refresh |= ctx.lock.stop();
                    }
                    continue;
                }
                ctx.keep_awake();
                refresh |= released(ctx, shell, keys, button, now);
            }
        }
    }
    if refresh {
        shell.refresh(ctx);
    }
    true
}

/// Relâchement d'un bouton, comme la boucle de `storyTeller.c`.
fn released(ctx: &mut Ctx, shell: &mut Shell, keys: &mut Keys, button: Button, now: i64) -> bool {
    let mut refresh = false;
    let debounced = now - keys.last_action > DEBOUNCE_MS;
    let action: Option<fn(&mut Shell, &mut Ctx)> = match button {
        Button::Left => Some(Shell::previous),
        Button::Right => Some(Shell::next),
        Button::Up => Some(Shell::up),
        Button::Down => Some(Shell::down),
        Button::Start | Button::Select => Some(Shell::pause),
        Button::A | Button::B => Some(Shell::ok),
        Button::X | Button::Y => Some(Shell::home),
        _ => None,
    };
    if let Some(action) = action.filter(|_| debounced) {
        keys.last_action = now;
        action(shell, ctx);
    }
    match button {
        Button::Power => keys.power_since = None,
        Button::Menu => {
            if !keys.menu_prevent_default {
                shell.menu(ctx);
            }
            keys.menu_pressed = false;
            keys.menu_prevent_default = false;
            refresh |= ctx.lock.stop();
        }
        _ => {}
    }
    // Menu maintenu + volume ou L2/R2 : luminosité ; sinon volume.
    let step = match button {
        Button::L2 | Button::VolumeDown => -1,
        Button::R2 | Button::VolumeUp => 1,
        _ => return refresh,
    };
    if keys.menu_pressed {
        ctx.brightness_step(step);
        ctx.lock.stop();
        keys.menu_prevent_default = true;
        true
    } else if matches!(button, Button::VolumeDown | Button::VolumeUp) {
        ctx.volume_step(step);
        true
    } else {
        refresh
    }
}
