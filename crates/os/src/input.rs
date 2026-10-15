//! Boutons : `/dev/input/event0` sur la console, clavier SDL sur le Mac.

use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;

use crate::log;
use crate::sdl::{self, Sdl};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    L1,
    R1,
    L2,
    R2,
    Select,
    Start,
    Menu,
    Power,
    VolumeUp,
    VolumeDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Pressed(Button),
    Released(Button),
    /// Fermeture de la fenêtre du simulateur.
    Quit,
}

pub struct Input {
    device: Option<File>,
}

const EV_KEY: u16 = 1;

/// `struct input_event` de `linux/input.h` (16 octets sur ARMv7).
#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

/// Codes `linux/input.h` des boutons de la Miyoo Mini (voir `keymap_hw.h`).
fn button_from_code(code: u16) -> Option<Button> {
    Some(match code {
        103 => Button::Up,
        108 => Button::Down,
        105 => Button::Left,
        106 => Button::Right,
        57 => Button::A,
        29 => Button::B,
        42 => Button::X,
        56 => Button::Y,
        18 => Button::L1,
        20 => Button::R1,
        15 => Button::L2,
        14 => Button::R2,
        97 => Button::Select,
        28 => Button::Start,
        1 => Button::Menu,
        116 => Button::Power,
        115 => Button::VolumeUp,
        114 => Button::VolumeDown,
        _ => return None,
    })
}

/// Clavier du simulateur : flèches, Entrée = A, Retour arrière = Y (home),
/// Espace = Start (pause), Échap = Menu, +/- = volume, P = marche/arrêt.
fn button_from_keycode(keycode: i32) -> Option<Button> {
    Some(match keycode {
        0x4000_0052 => Button::Up,
        0x4000_0051 => Button::Down,
        0x4000_0050 => Button::Left,
        0x4000_004F => Button::Right,
        13 => Button::A,
        8 => Button::Y,
        32 => Button::Start,
        27 => Button::Menu,
        61 | 43 => Button::VolumeUp,
        45 => Button::VolumeDown,
        108 => Button::L2,
        114 => Button::R2,
        112 => Button::Power,
        _ => return None,
    })
}

impl Input {
    pub fn open(path: &str) -> Input {
        use std::os::unix::fs::OpenOptionsExt;
        let device = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| log!("entrée {path} indisponible : {e}"))
            .ok();
        Input { device }
    }

    /// Simulateur : pas de périphérique, on lit le clavier SDL.
    pub fn desktop() -> Input {
        Input { device: None }
    }

    /// Attend au plus `timeout_ms` et renvoie les événements reçus.
    pub fn wait(&mut self, sdl: &mut Sdl, timeout_ms: i32, events: &mut Vec<Event>) {
        match self.device.as_mut() {
            Some(device) => read_device(device, timeout_ms, events),
            None => {
                let mut timeout = timeout_ms;
                while let Some(event) = sdl.wait_event(timeout) {
                    timeout = 0;
                    match event.kind {
                        sdl::SDL_QUIT => events.push(Event::Quit),
                        sdl::SDL_KEYDOWN | sdl::SDL_KEYUP if event.repeat == 0 => {
                            if let Some(button) = button_from_keycode(event.keycode) {
                                events.push(if event.kind == sdl::SDL_KEYDOWN {
                                    Event::Pressed(button)
                                } else {
                                    Event::Released(button)
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn read_device(device: &mut File, timeout_ms: i32, events: &mut Vec<Event>) {
    let mut poll = libc::pollfd { fd: device.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: un seul pollfd valide, durée bornée.
    if unsafe { libc::poll(&mut poll, 1, timeout_ms) } <= 0 {
        return;
    }
    const SIZE: usize = std::mem::size_of::<InputEvent>();
    let mut buffer = [0u8; SIZE * 32];
    while let Ok(n) = device.read(&mut buffer) {
        if n == 0 {
            break;
        }
        for chunk in buffer[..n].as_chunks::<SIZE>().0 {
            // SAFETY: le noyau écrit des `input_event` complets ; lecture non alignée.
            let ev: InputEvent = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast()) };
            if ev.kind != EV_KEY {
                continue;
            }
            let Some(button) = button_from_code(ev.code) else { continue };
            match ev.value {
                1 => events.push(Event::Pressed(button)),
                0 => events.push(Event::Released(button)),
                _ => {} // répétition automatique ignorée, comme Telmi
            }
        }
        if n < buffer.len() {
            break;
        }
    }
}
