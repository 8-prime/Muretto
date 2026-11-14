//! The installed game and the session being prepared for it. One instance,
//! shared by every screen.

use std::sync::Arc;

use gpui_kit::{Context, SharedString};
use muretto_core::{
    Car, Install, Library, SessionConfig, Skin, Track, TrackLayout, launch,
    packages::csp,
    session::{Assists, SessionMode},
    settings::{self, Config},
};

#[derive(Debug, Clone, PartialEq)]
pub enum Launch {
    Idle,
    Started,
    Failed(SharedString),
}

pub struct Game {
    pub install: Install,
    pub library: Arc<Library>,
    pub launch: Launch,
    /// Custom Shaders Patch as last seen in the game folder.
    pub patch: Option<csp::Installed>,
    /// Muretto's fix that lets the patch work under Proton.
    pub proton_fix: csp::ProtonFix,
    /// Goes up whenever the mods in the game folder may have changed, so
    /// views that read them from disk know when to look again.
    pub installed_revision: u64,
    /// The game's own settings files.
    pub config: Config,
    /// Why the last change to them could not be written.
    pub config_error: Option<SharedString>,
    session: SessionConfig,
}

impl Game {
    /// `None` when the install has no car or track that can be driven.
    pub fn new(install: Install, library: Library) -> Option<Self> {
        let mut session = SessionConfig::first_run(&library)?;
        let config = Config::load(&install);
        // Assists set in the game's files last time carry over.
        if config.has_own(settings::ASSISTS) {
            session.assists = Assists::from_ini(config.file(settings::ASSISTS), &session.assists);
        }
        Some(Self {
            config,
            config_error: None,
            patch: csp::installed(&install),
            proton_fix: csp::proton_fix(&install),
            installed_revision: 0,
            install,
            library: Arc::new(library),
            launch: Launch::Idle,
            session,
        })
    }

    /// Looks at the game folder again, after something was installed,
    /// removed or switched.
    pub fn refresh_installed(&mut self, cx: &mut Context<Self>) {
        self.patch = csp::installed(&self.install);
        self.proton_fix = csp::proton_fix(&self.install);
        self.installed_revision += 1;
        cx.notify();
    }

    /// Takes a fresh reading of the game's content, after cars or tracks
    /// were added.
    pub fn set_library(&mut self, library: Library, cx: &mut Context<Self>) {
        self.library = Arc::new(library);
        self.installed_revision += 1;
        cx.notify();
    }

    /// Changes the game's settings files. The change is written at once.
    pub fn configure(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Config, &Install) -> muretto_core::Result<()>,
    ) -> muretto_core::Result<()> {
        let result = change(&mut self.config, &self.install);
        cx.notify();
        result
    }

    /// Changes the assists, and keeps them for next time in the game's
    /// `assists.ini`.
    pub fn set_assists(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Assists),
    ) -> muretto_core::Result<()> {
        self.edit(cx, |session| change(&mut session.assists));
        let ini = self.session.assists.to_ini();
        self.config.replace(settings::ASSISTS, ini)
    }

    pub fn session(&self) -> &SessionConfig {
        &self.session
    }

    pub fn car(&self) -> &Car {
        self.library
            .car(&self.session.car_id)
            .expect("the session's car is in the library")
    }

    pub fn skin(&self) -> Option<&Skin> {
        self.car().skin(&self.session.skin_id)
    }

    pub fn track(&self) -> &Track {
        self.library
            .track(&self.session.track_id)
            .expect("the session's track is in the library")
    }

    pub fn layout(&self) -> &TrackLayout {
        self.track().layout(self.session.layout_id.as_deref())
    }

    /// Changes the session, then brings dependent values back into range.
    pub fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut SessionConfig)) {
        change(&mut self.session);
        let max_opponents = self.session.max_opponents(&self.library);
        let session = &mut self.session;
        session.laps = session.laps.clamp(1, 999);
        session.opponents = session.opponents.min(max_opponents);
        if session.mode.has_opponents() && max_opponents > 0 {
            session.opponents = session.opponents.max(1);
        }
        session.start_position = session.start_position.clamp(1, session.opponents + 1);
        self.launch = Launch::Idle;
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: SessionMode, cx: &mut Context<Self>) {
        self.edit(cx, |session| session.mode = mode);
    }

    pub fn set_car(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(car) = self.library.car(id).filter(|car| car.owned) else {
            return;
        };
        let skin = car.skins.first().map(|s| s.id.clone()).unwrap_or_default();
        let id = car.id.clone();
        self.edit(cx, |session| {
            session.car_id = id;
            session.skin_id = skin;
        });
    }

    pub fn set_track(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(track) = self.library.track(id).filter(|track| track.owned) else {
            return;
        };
        let layout = track.layouts[0].id.clone();
        let id = track.id.clone();
        self.edit(cx, |session| {
            session.track_id = id;
            session.layout_id = layout;
        });
    }

    /// Writes the session and starts the simulator.
    pub fn drive(&mut self, cx: &mut Context<Self>) {
        self.launch = match launch::start(&self.install, &self.library, &self.session) {
            Ok(mut child) => {
                // Collect the exit status when the game closes, however long
                // that takes, without holding an executor thread.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                Launch::Started
            }
            Err(error) => Launch::Failed(error.to_string().into()),
        };
        cx.notify();
    }
}
