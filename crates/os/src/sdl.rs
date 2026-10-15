//! Liaison minimale avec SDL2.
//!
//! Sur la console, on utilise la `libSDL2.so` livrée avec Telmi OS (pilotes
//! `mmiyoo` pour l'écran et le son), en reproduisant exactement ses appels :
//! texture RGB565 alimentée par un tampon 32 bits, rendu accéléré avec vsync,
//! ancienne API audio `SDL_OpenAudio`. Sur le Mac, la SDL2 de Homebrew sert de
//! simulateur.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr;

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;

const SDL_INIT_AUDIO: u32 = 0x10;
const SDL_INIT_VIDEO: u32 = 0x20;
const SDL_GL_CONTEXT_MAJOR_VERSION: c_int = 17;
const SDL_GL_CONTEXT_MINOR_VERSION: c_int = 18;
const SDL_GL_CONTEXT_PROFILE_MASK: c_int = 21;
const SDL_GL_CONTEXT_PROFILE_ES: c_int = 0x4;
const SDL_WINDOW_SHOWN: u32 = 0x4;
const SDL_RENDERER_ACCELERATED: u32 = 0x2;
const SDL_RENDERER_PRESENTVSYNC: u32 = 0x4;
const SDL_TEXTUREACCESS_STREAMING: c_int = 1;
#[cfg(target_arch = "arm")]
const TEXTURE_FORMAT: u32 = 0x1515_1002; // SDL_PIXELFORMAT_RGB565, comme Telmi
#[cfg(not(target_arch = "arm"))]
const TEXTURE_FORMAT: u32 = 0x1636_2004; // SDL_PIXELFORMAT_ARGB8888
const AUDIO_S16LSB: u16 = 0x8010;

pub const SDL_QUIT: u32 = 0x100;
pub const SDL_KEYDOWN: u32 = 0x300;
pub const SDL_KEYUP: u32 = 0x301;

#[repr(C)]
struct Opaque {
    _private: [u8; 0],
}

pub type AudioCallback = extern "C" fn(userdata: *mut c_void, stream: *mut u8, len: c_int);

#[repr(C)]
struct AudioSpec {
    freq: c_int,
    format: u16,
    channels: u8,
    silence: u8,
    samples: u16,
    padding: u16,
    size: u32,
    callback: Option<AudioCallback>,
    userdata: *mut c_void,
}

/// `SDL_Event` (56 octets), lu seulement pour le clavier du simulateur.
#[repr(C)]
pub struct Event {
    pub kind: u32,
    timestamp: u32,
    window_id: u32,
    state: u8,
    pub repeat: u8,
    padding: [u8; 2],
    scancode: i32,
    pub keycode: i32,
    rest: [u8; 32],
}

#[link(name = "SDL2")]
extern "C" {
    fn SDL_Init(flags: u32) -> c_int;
    fn SDL_Quit();
    fn SDL_GetError() -> *const c_char;
    fn SDL_GL_SetAttribute(attr: c_int, value: c_int) -> c_int;
    fn SDL_CreateWindow(title: *const c_char, x: c_int, y: c_int, w: c_int, h: c_int, flags: u32) -> *mut Opaque;
    fn SDL_DestroyWindow(window: *mut Opaque);
    fn SDL_CreateRenderer(window: *mut Opaque, index: c_int, flags: u32) -> *mut Opaque;
    fn SDL_DestroyRenderer(renderer: *mut Opaque);
    fn SDL_CreateTexture(renderer: *mut Opaque, format: u32, access: c_int, w: c_int, h: c_int) -> *mut Opaque;
    fn SDL_DestroyTexture(texture: *mut Opaque);
    fn SDL_UpdateTexture(texture: *mut Opaque, rect: *const c_void, pixels: *const c_void, pitch: c_int) -> c_int;
    fn SDL_RenderClear(renderer: *mut Opaque) -> c_int;
    fn SDL_RenderCopy(renderer: *mut Opaque, texture: *mut Opaque, src: *const c_void, dst: *const c_void) -> c_int;
    fn SDL_RenderPresent(renderer: *mut Opaque);
    fn SDL_OpenAudio(desired: *mut AudioSpec, obtained: *mut AudioSpec) -> c_int;
    fn SDL_PauseAudio(pause_on: c_int);
    fn SDL_CloseAudio();
    fn SDL_WaitEventTimeout(event: *mut Event, timeout: c_int) -> c_int;
}

fn last_error() -> String {
    // SAFETY: SDL_GetError renvoie toujours une chaîne C valide, éventuellement vide.
    unsafe { CStr::from_ptr(SDL_GetError()).to_string_lossy().into_owned() }
}

/// Contexte SDL : fenêtre plein écran 640×480 et sortie audio.
pub struct Sdl {
    window: *mut Opaque,
    renderer: *mut Opaque,
    texture: *mut Opaque,
    audio_open: bool,
}

impl Sdl {
    pub fn new() -> Result<Sdl, String> {
        // SAFETY: appels d'initialisation SDL dans l'ordre de Telmi ; chaque
        // pointeur renvoyé est vérifié avant usage.
        unsafe {
            if SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO) != 0 {
                return Err(format!("SDL_Init : {}", last_error()));
            }
            // OpenGL ES 2.0, comme Telmi pour le pilote mmiyoo ; absent sur macOS.
            if cfg!(target_arch = "arm") {
                SDL_GL_SetAttribute(SDL_GL_CONTEXT_PROFILE_MASK, SDL_GL_CONTEXT_PROFILE_ES);
                SDL_GL_SetAttribute(SDL_GL_CONTEXT_MAJOR_VERSION, 2);
                SDL_GL_SetAttribute(SDL_GL_CONTEXT_MINOR_VERSION, 0);
            }
            let mut sdl = Sdl {
                window: ptr::null_mut(),
                renderer: ptr::null_mut(),
                texture: ptr::null_mut(),
                audio_open: false,
            };
            sdl.window = SDL_CreateWindow(
                c"Telmi".as_ptr(),
                0,
                0,
                WIDTH as c_int,
                HEIGHT as c_int,
                SDL_WINDOW_SHOWN,
            );
            if sdl.window.is_null() {
                return Err(format!("SDL_CreateWindow : {}", last_error()));
            }
            sdl.renderer = SDL_CreateRenderer(
                sdl.window,
                -1,
                SDL_RENDERER_ACCELERATED | SDL_RENDERER_PRESENTVSYNC,
            );
            if sdl.renderer.is_null() {
                return Err(format!("SDL_CreateRenderer : {}", last_error()));
            }
            sdl.texture = SDL_CreateTexture(
                sdl.renderer,
                TEXTURE_FORMAT,
                SDL_TEXTUREACCESS_STREAMING,
                WIDTH as c_int,
                HEIGHT as c_int,
            );
            if sdl.texture.is_null() {
                return Err(format!("SDL_CreateTexture : {}", last_error()));
            }
            Ok(sdl)
        }
    }

    /// Affiche une image 640×480 au format 0x00RRGGBB.
    pub fn present(&mut self, pixels: &[u32]) {
        assert_eq!(pixels.len(), WIDTH * HEIGHT);
        // SAFETY: le tampon fait exactement WIDTH×HEIGHT pixels de 4 octets,
        // ce qui correspond au pas transmis.
        unsafe {
            SDL_RenderClear(self.renderer);
            SDL_UpdateTexture(self.texture, ptr::null(), pixels.as_ptr().cast(), (WIDTH * 4) as c_int);
            SDL_RenderCopy(self.renderer, self.texture, ptr::null(), ptr::null());
            SDL_RenderPresent(self.renderer);
        }
    }

    /// Ouvre la sortie audio 44,1 kHz stéréo 16 bits.
    ///
    /// `userdata` doit rester valide jusqu'à la destruction de `Sdl`.
    pub fn open_audio(&mut self, callback: AudioCallback, userdata: *mut c_void) -> Result<(), String> {
        let mut spec = AudioSpec {
            freq: crate::audio::SAMPLE_RATE as c_int,
            format: AUDIO_S16LSB,
            channels: 2,
            silence: 0,
            samples: 4096,
            padding: 0,
            size: 0,
            callback: Some(callback),
            userdata,
        };
        // SAFETY: `spec` est entièrement initialisée ; SDL copie la structure.
        unsafe {
            if SDL_OpenAudio(&mut spec, ptr::null_mut()) < 0 {
                return Err(format!("SDL_OpenAudio : {}", last_error()));
            }
            SDL_PauseAudio(0);
        }
        self.audio_open = true;
        Ok(())
    }

    /// Attend un événement SDL (clavier du simulateur) au plus `timeout_ms`.
    pub fn wait_event(&mut self, timeout_ms: i32) -> Option<Event> {
        // SAFETY: Event a la taille et l'alignement d'un SDL_Event.
        unsafe {
            let mut event: Event = std::mem::zeroed();
            (SDL_WaitEventTimeout(&mut event, timeout_ms) == 1).then_some(event)
        }
    }
}

impl Drop for Sdl {
    fn drop(&mut self) {
        // SAFETY: chaque ressource n'est détruite qu'une fois, si elle existe.
        unsafe {
            if self.audio_open {
                SDL_CloseAudio();
            }
            if !self.texture.is_null() {
                SDL_DestroyTexture(self.texture);
            }
            if !self.renderer.is_null() {
                SDL_DestroyRenderer(self.renderer);
            }
            if !self.window.is_null() {
                SDL_DestroyWindow(self.window);
            }
            SDL_Quit();
        }
    }
}

const _: () = assert!(std::mem::size_of::<Event>() == 56);
