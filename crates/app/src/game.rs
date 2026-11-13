//! The installed game and the session being prepared for it. One instance,
//! shared by every screen.

use std::sync::Arc;

use gpui_kit::{Context, SharedString};
use muretto_core::{
    Car, Install, Library, SessionConfig, Skin, Track, TrackLayout, launch, session::SessionMode,
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
    session: SessionConfig,
}

impl Game {
    /// `None` when the install has no car or track that can be driven.
    pub fn new(install: Install, library: Library) -> Option<Self> {
        let session = SessionConfig::first_run(&library)?;
        Some(Self {
            install,
            library: Arc::new(library),
            launch: Launch::Idle,
            session,
        })
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
