//! Sélecteur d'applications et routage des boutons, portage de `app_selector.h`.

mod music;
mod stories;

use telmi_story::save::{self, SessionState, APP_MUSIC, APP_NIGHT_MODE, APP_STORIES};

use crate::log;
use crate::ui::Ctx;
use music::Music;
use stories::Stories;

const APPS: [i64; 3] = [APP_STORIES, APP_MUSIC, APP_NIGHT_MODE];

enum Opened {
    None,
    Stories(Box<Stories>),
    Music(Box<Music>),
}

pub struct Shell {
    app: i64,
    opened: Opened,
    /// Applications déjà chargées, gardées pour une réouverture instantanée.
    stories: Option<Box<Stories>>,
    night: Option<Box<Stories>>,
    music: Option<Box<Music>>,
}

macro_rules! route {
    ($shell:expr, $ctx:expr, $method:ident) => {
        match &mut $shell.opened {
            Opened::Stories(app) => app.$method($ctx),
            Opened::Music(app) => app.$method($ctx),
            Opened::None => {}
        }
    };
}

impl Shell {
    /// Rouvre l'application de la dernière session, sinon affiche le sélecteur.
    pub fn start(ctx: &mut Ctx) -> Shell {
        let path = ctx.paths.session();
        let session = save::read_session(&path);
        // Consommée une seule fois, comme Telmi : un état qui ferait planter
        // ne peut pas relancer la même panne au démarrage suivant.
        let _ = std::fs::remove_file(&path);
        let mut shell = Shell { app: APP_STORIES, opened: Opened::None, stories: None, night: None, music: None };
        match session {
            Some(state) if APPS.contains(&state.app()) => {
                shell.app = state.app();
                shell.open(ctx, Some(state));
            }
            _ => shell.show(ctx),
        }
        shell
    }

    fn night_mode_enabled(ctx: &Ctx) -> bool {
        !ctx.params.story_disable_night_mode
    }

    fn show(&mut self, ctx: &mut Ctx) {
        let image = match self.app {
            APP_MUSIC => "selectMusic.png",
            APP_NIGHT_MODE => "selectNightMode.png",
            _ => "selectStories.png",
        };
        ctx.show_res(image);
        ctx.set_screen(true);
        ctx.autosleep_default();
    }

    fn cycle(&mut self, ctx: &mut Ctx, direction: isize) {
        let mut i = APPS.iter().position(|&a| a == self.app).unwrap_or(0) as isize;
        loop {
            i = (i + direction).rem_euclid(APPS.len() as isize);
            if APPS[i as usize] != APP_NIGHT_MODE || Self::night_mode_enabled(ctx) {
                break;
            }
        }
        self.app = APPS[i as usize];
        self.show(ctx);
    }

    fn open(&mut self, ctx: &mut Ctx, session: Option<SessionState>) {
        self.opened = match self.app {
            APP_MUSIC => Opened::Music(match self.music.take() {
                Some(mut music) => {
                    music.reopen(ctx);
                    music
                }
                None => Box::new(Music::new(ctx, session)),
            }),
            APP_NIGHT_MODE => Opened::Stories(match self.night.take() {
                Some(mut night) => {
                    night.reopen(ctx);
                    night
                }
                None => Box::new(Stories::new(ctx, true, session)),
            }),
            _ => Opened::Stories(match self.stories.take() {
                Some(mut stories) => {
                    stories.reopen(ctx);
                    stories
                }
                None => Box::new(Stories::new(ctx, false, session)),
            }),
        };
    }

    /// Range l'application ouverte pour la retrouver plus tard.
    fn close(&mut self) {
        match std::mem::replace(&mut self.opened, Opened::None) {
            Opened::Stories(app) if self.app == APP_NIGHT_MODE => self.night = Some(app),
            Opened::Stories(app) => self.stories = Some(app),
            Opened::Music(app) => self.music = Some(app),
            Opened::None => {}
        }
    }

    fn is_open(&self) -> bool {
        !matches!(self.opened, Opened::None)
    }

    pub fn update(&mut self, ctx: &mut Ctx) {
        route!(self, ctx, update);
    }

    pub fn refresh(&mut self, ctx: &mut Ctx) {
        if self.is_open() {
            route!(self, ctx, refresh);
        } else {
            self.show(ctx);
        }
    }

    pub fn menu(&mut self, ctx: &mut Ctx) {
        route!(self, ctx, menu);
    }

    pub fn previous(&mut self, ctx: &mut Ctx) {
        if self.is_open() {
            route!(self, ctx, previous);
        } else {
            self.cycle(ctx, -1);
        }
    }

    pub fn next(&mut self, ctx: &mut Ctx) {
        if self.is_open() {
            route!(self, ctx, next);
        } else {
            self.cycle(ctx, 1);
        }
    }

    pub fn up(&mut self, ctx: &mut Ctx) {
        route!(self, ctx, up);
    }

    pub fn down(&mut self, ctx: &mut Ctx) {
        route!(self, ctx, down);
    }

    pub fn pause(&mut self, ctx: &mut Ctx) {
        route!(self, ctx, pause);
    }

    pub fn ok(&mut self, ctx: &mut Ctx) {
        if self.is_open() {
            route!(self, ctx, ok);
        } else {
            self.open(ctx, None);
        }
    }

    pub fn home(&mut self, ctx: &mut Ctx) {
        let leave = match &mut self.opened {
            Opened::Stories(app) => app.home(ctx),
            Opened::Music(app) => app.home(ctx),
            Opened::None => false,
        };
        if leave {
            self.close();
            self.show(ctx);
        }
    }

    /// Enregistre l'application ouverte pour la rouvrir au prochain démarrage.
    pub fn save(&mut self, ctx: &mut Ctx) {
        let state = match &mut self.opened {
            Opened::Stories(app) => app.session(ctx),
            Opened::Music(app) => app.session(ctx),
            Opened::None => None,
        };
        if let Some(state) = state {
            let path = ctx.paths.session();
            if let Err(e) = save::write_session(&path, &state) {
                log!("sauvegarde de session : {e}");
            }
        }
    }
}
