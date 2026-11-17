//! Host: the player's server hosts, machines running Muretto's host agent
//! next to AssettoServer. Each is added with its address and the token its
//! owner handed out. A host keeps a library of presets; the player looks
//! through them, writes new ones, changes them and activates one, which
//! restarts the server with it. Content the host lacks is uploaded from the
//! player's own game folder on the way.
//!
//! Shown only with Settings › Launcher › General › Server hosting on.

use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui_kit::{
    App, Context, Div, Entity, FontWeight, IntoElement, Render, SharedString, Window,
    component::input::{Input, InputState},
    div,
    prelude::*,
    px, rgb,
};
use muretto_core::{
    Library,
    host::{
        CONFLICT, HostError, HostResult, Hosts, KeptHost, Synced, UNPROCESSABLE,
        protocol::{
            ServerState, Status,
            presets::{
                AiMode, Entry, FieldError, Preset, PresetId, PresetSummary, Race, RaceJoin,
                RaceLength, Session, TrackRef,
            },
        },
        sync,
    },
};

use crate::{
    game::Game,
    job::Job,
    theme::{GAP, PAGE, RADIUS, color, font},
    ui::{
        ChoiceField, NumberField, badge, button, display, label, mono, muted, panel, stat_strip,
        switch,
    },
};

/// What a host said when last asked.
#[derive(Default)]
struct HostView {
    loading: bool,
    status: Option<Status>,
    presets: Option<Vec<PresetSummary>>,
    error: Option<String>,
}

/// A step that is asked about once before it is taken.
#[derive(Clone, PartialEq)]
enum Confirm {
    RemoveHost(String),
    DeletePreset(PresetId),
    /// Drivers are on the server; the reason is the host's.
    Force(PresetId, String),
}

pub struct HostScreen {
    game: Entity<Game>,
    hosts: Hosts,
    /// The URL of the host shown.
    selected: Option<String>,
    /// The form to add a host is open.
    adding: bool,
    name_input: Entity<InputState>,
    address_input: Entity<InputState>,
    token_input: Entity<InputState>,
    views: HashMap<String, HostView>,
    editor: Option<Editor>,
    /// Content going up to a host.
    job: Option<Job>,
    /// A request that is under way, said in words.
    busy: Option<SharedString>,
    /// Something to tell the player, and whether it is a problem.
    note: Option<(SharedString, bool)>,
    confirm: Option<Confirm>,
}

impl HostScreen {
    pub fn new(
        game: Entity<Game>,
        hosts: Hosts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let name_input = input("Name, such as Club server", window, cx);
        let address_input = input("https://host.example or http://10.0.0.5:3200", window, cx);
        let token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Token from the host's owner")
                .masked(true)
        });
        Self {
            game,
            selected: hosts.hosts.first().map(|host| host.url.clone()),
            adding: hosts.hosts.is_empty(),
            hosts,
            name_input,
            address_input,
            token_input,
            views: HashMap::new(),
            editor: None,
            job: None,
            busy: None,
            note: None,
            confirm: None,
        }
    }

    pub fn hosts(&self) -> &[KeptHost] {
        &self.hosts.hosts
    }

    /// The presets of the host shown, once it has listed them.
    pub fn presets(&self) -> Option<&[PresetSummary]> {
        self.view()?.presets.as_deref()
    }

    /// The state of the host shown, once it has said.
    pub fn status(&self) -> Option<&Status> {
        self.view()?.status.as_ref()
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_ref().map(|(text, _)| text.as_ref())
    }

    /// Whether a request, upload or save is under way.
    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
            || self.job.is_some()
            || self.view().is_some_and(|view| view.loading)
            || self.editor.as_ref().is_some_and(|editor| editor.saving)
    }

    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }

    fn view(&self) -> Option<&HostView> {
        self.views.get(self.selected.as_ref()?)
    }

    fn host(&self, url: &str) -> Option<&KeptHost> {
        self.hosts.hosts.iter().find(|host| host.url == url)
    }

    fn selected_host(&self) -> Option<KeptHost> {
        self.host(self.selected.as_deref()?).cloned()
    }

    /// The section was opened: brings the host shown up to date.
    pub fn visit(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.selected.clone() {
            self.refresh(&url, cx);
        }
    }

    fn select(&mut self, url: String, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&url) {
            self.editor = None;
            self.confirm = None;
            self.note = None;
        }
        self.adding = false;
        self.selected = Some(url.clone());
        self.refresh(&url, cx);
        cx.notify();
    }

    /// Asks the host for its state and presets.
    fn refresh(&mut self, url: &str, cx: &mut Context<Self>) {
        let Some(host) = self.host(url).cloned() else {
            return;
        };
        let view = self.views.entry(host.url.clone()).or_default();
        if view.loading {
            return;
        }
        view.loading = true;
        let client = host.client();
        let asking = cx
            .background_executor()
            .spawn(async move { client.status().and_then(|s| Ok((s, client.presets()?))) });
        cx.spawn(async move |this, cx| {
            let answer = asking.await;
            this.update(cx, |this, cx| {
                let view = this.views.entry(host.url).or_default();
                view.loading = false;
                match answer {
                    Ok((status, presets)) => {
                        view.status = Some(status);
                        view.presets = Some(presets);
                        view.error = None;
                    }
                    Err(error) => view.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Keeps a host and shows it; `false`, with a note saying why, when the
    /// address or token will not do.
    pub fn add(&mut self, name: &str, address: &str, token: &str, cx: &mut Context<Self>) -> bool {
        if token.trim().is_empty() {
            self.note = Some(("Put in the token the host's owner gave you.".into(), true));
            cx.notify();
            return false;
        }
        let url = match self.hosts.add(name, address, token) {
            Ok(host) => host.url.clone(),
            Err(error) => {
                self.note = Some((error.to_string().into(), true));
                cx.notify();
                return false;
            }
        };
        self.views.remove(&url);
        self.note = None;
        self.select(url, cx);
        true
    }

    fn add_from_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let read = |input: &Entity<InputState>, cx: &App| input.read(cx).value().to_string();
        let (name, address, token) = (
            read(&self.name_input, cx),
            read(&self.address_input, cx),
            read(&self.token_input, cx),
        );
        if self.add(&name, &address, &token, cx) {
            for input in [&self.name_input, &self.address_input, &self.token_input] {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
    }

    /// Whether presets put on the host at `url` take the track's models.
    pub fn set_track_models(&mut self, url: &str, on: bool, cx: &mut Context<Self>) {
        self.note = self
            .hosts
            .set_track_models(url, on)
            .err()
            .map(|error| (format!("Could not keep the setting: {error}").into(), true));
        cx.notify();
    }

    fn remove_host(&mut self, url: &str, cx: &mut Context<Self>) {
        let ask = Confirm::RemoveHost(url.to_owned());
        if self.confirm.as_ref() != Some(&ask) {
            self.confirm = Some(ask);
            cx.notify();
            return;
        }
        self.confirm = None;
        self.editor = None;
        self.note = self
            .hosts
            .remove(url)
            .err()
            .map(|error| (format!("Could not forget the host: {error}").into(), true));
        self.views.remove(url);
        self.selected = self.hosts.hosts.first().map(|host| host.url.clone());
        self.adding = self.hosts.hosts.is_empty();
        if let Some(url) = self.selected.clone() {
            self.refresh(&url, cx);
        }
        cx.notify();
    }

    pub fn new_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.selected_host() else {
            return;
        };
        let game = self.game.read(cx);
        let library = game.library.clone();
        let session = game.session();
        let track = TrackRef {
            id: session.track_id.clone(),
            layout: session.layout_id.clone(),
        };
        let name = library
            .tracks
            .iter()
            .find(|t| t.id == track.id)
            .map_or_else(|| track.id.clone(), |t| t.name.clone());
        let preset = Preset::new(name, track, vec![fresh_entry(&session.car_id)]);
        self.confirm = None;
        self.note = None;
        self.editor = Some(Editor::new(
            host.url,
            PresetId::new(),
            None,
            preset,
            &library,
            window,
            cx,
        ));
        cx.notify();
    }

    /// Fetches a preset whole and opens it in the editor.
    pub fn open_preset(&mut self, id: PresetId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.selected_host() else {
            return;
        };
        self.busy = Some("Opening the preset…".into());
        let client = host.client();
        let asking = cx
            .background_executor()
            .spawn(async move { client.preset(id) });
        cx.spawn_in(window, async move |this, cx| {
            let answer = asking.await;
            this.update_in(cx, |this, window, cx| {
                this.busy = None;
                if this.selected.as_ref() != Some(&host.url) {
                    return;
                }
                match answer {
                    Ok(stored) => {
                        let library = this.game.read(cx).library.clone();
                        this.confirm = None;
                        this.note = None;
                        this.editor = Some(Editor::new(
                            host.url,
                            stored.id,
                            Some(stored.revision),
                            stored.preset,
                            &library,
                            window,
                            cx,
                        ));
                    }
                    Err(error) => this.note = Some((error.to_string().into(), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        cx.notify();
    }

    /// Sends the preset in the editor to its host.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self
            .editor
            .as_ref()
            .and_then(|editor| self.host(&editor.url))
            .cloned()
        else {
            return;
        };
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.saving {
            return;
        }
        let preset = editor.preset(cx);
        editor.errors = preset.validate();
        if !editor.errors.is_empty() {
            self.note = Some(("The preset has mistakes; see below.".into(), true));
            cx.notify();
            return;
        }
        editor.saving = true;
        let (id, revision) = (editor.id, editor.revision);
        let client = host.client();
        let saving = cx.background_executor().spawn(async move {
            match revision {
                Some(revision) => client.save(id, revision, &preset),
                None => client.create(id, &preset),
            }
        });
        cx.spawn(async move |this, cx| {
            let answer = saving.await;
            this.update(cx, |this, cx| {
                let Some(editor) = this.editor.as_mut().filter(|editor| editor.id == id) else {
                    return;
                };
                editor.saving = false;
                match answer {
                    Ok(stored) => {
                        this.editor = None;
                        this.note = Some((
                            format!("Saved {} on {}.", stored.preset.name, host.name).into(),
                            false,
                        ));
                        this.refresh(&host.url, cx);
                    }
                    Err(error) if error.status() == Some(CONFLICT) && revision.is_some() => {
                        this.note = Some((
                            format!(
                                "{error} Close the editor and open the preset again to see what changed."
                            )
                            .into(),
                            true,
                        ));
                    }
                    Err(error) => {
                        editor.errors = error.fields().to_vec();
                        this.note = Some((error.to_string().into(), true));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn delete_preset(&mut self, id: PresetId, cx: &mut Context<Self>) {
        let ask = Confirm::DeletePreset(id);
        if self.confirm.as_ref() != Some(&ask) {
            self.confirm = Some(ask);
            cx.notify();
            return;
        }
        self.confirm = None;
        let Some(host) = self.selected_host() else {
            return;
        };
        self.busy = Some("Deleting the preset…".into());
        let client = host.client();
        let deleting = cx
            .background_executor()
            .spawn(async move { client.delete(id) });
        cx.spawn(async move |this, cx| {
            let answer = deleting.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                this.note = answer.err().map(|error| (error.to_string().into(), true));
                this.refresh(&host.url, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Makes the preset the one the server runs. With drivers on the
    /// server this asks first; content the host lacks goes up on the way.
    pub fn activate(&mut self, id: PresetId, force: bool, cx: &mut Context<Self>) {
        let Some(host) = self.selected_host() else {
            return;
        };
        if self.busy.is_some() || self.job.is_some() {
            return;
        }
        self.confirm = None;
        self.note = None;
        self.busy = Some("Activating: the server restarts…".into());
        let client = host.client();
        let activating = cx
            .background_executor()
            .spawn(async move { client.activate(id, force) });
        cx.spawn(async move |this, cx| {
            let answer = activating.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                this.activated(&host, id, force, answer, None, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn activated(
        &mut self,
        host: &KeptHost,
        id: PresetId,
        force: bool,
        answer: HostResult<Status>,
        synced: Option<Synced>,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(status) => {
                let name = status
                    .active
                    .as_ref()
                    .map_or("The preset".to_owned(), |active| active.name.clone());
                let uploaded = match synced {
                    Some(synced) if synced.uploaded > 0 => {
                        format!(" {} files of content went up first.", synced.uploaded)
                    }
                    _ => String::new(),
                };
                self.note = Some((
                    format!("{name} runs on {} now.{uploaded}", host.name).into(),
                    false,
                ));
                self.views.entry(host.url.clone()).or_default().status = Some(status);
                self.refresh(&host.url, cx);
            }
            Err(error) if error.status() == Some(CONFLICT) => {
                self.confirm = Some(Confirm::Force(id, error.to_string()));
            }
            Err(error) if error.status() == Some(UNPROCESSABLE) && synced.is_none() => {
                self.upload_and_activate(host.clone(), id, force, cx);
            }
            Err(error) => {
                let mut text = refusal(&error);
                if let Some(synced) = synced.filter(|synced| !synced.missing.is_empty()) {
                    text.push_str(&format!(
                        " Your game folder has no server files for {}, so they could not be uploaded.",
                        synced.missing.join(", ")
                    ));
                }
                self.note = Some((text.into(), true));
            }
        }
    }

    /// Puts the preset's content on the host from the game folder, then
    /// activates it again.
    fn upload_and_activate(
        &mut self,
        host: KeptHost,
        id: PresetId,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let install = self.game.read(cx).install.clone();
        let client = host.client();
        let title = format!("Uploading content to {}", host.name);
        self.job = Some(Job::start(
            title,
            install,
            cx,
            move |install, progress| {
                let run = || -> HostResult<(Synced, HostResult<Status>)> {
                    let stored = client.preset(id)?;
                    let synced = sync(
                        &client,
                        install,
                        &stored.preset,
                        host.track_models,
                        progress,
                    )?;
                    Ok((synced, client.activate(id, force)))
                };
                Ok(run())
            },
            move |this: &mut Self, result, cx| {
                this.job = None;
                match result {
                    Ok(Ok((synced, answer))) => {
                        this.activated(&host, id, force, answer, Some(synced), cx)
                    }
                    Ok(Err(HostError::Cancelled)) => {
                        this.note = Some(("Upload cancelled.".into(), false));
                    }
                    Ok(Err(error)) => this.note = Some((refusal(&error).into(), true)),
                    Err(error) => this.note = Some((error.to_string().into(), true)),
                }
            },
        ));
        cx.notify();
    }
}

/// A refusal with what the host said of each field.
fn refusal(error: &HostError) -> String {
    let mut text = error.to_string();
    for field in error.fields() {
        text.push_str(&format!(" {}", field.message));
    }
    text
}

fn fresh_entry(model: &str) -> Entry {
    Entry {
        model: model.to_owned(),
        skin: None,
        driver_name: None,
        team: None,
        guid: None,
        ballast_kg: 0,
        restrictor: 0,
        ai: AiMode::None,
    }
}

/// The preset being written or changed. Only the commonest settings have
/// a control; the rest keep what the preset had.
struct Editor {
    url: String,
    id: PresetId,
    /// `None` for a preset not saved yet.
    revision: Option<u64>,
    /// Where settings without a control come from.
    base: Preset,
    name: Entity<InputState>,
    password: Entity<InputState>,
    track: TrackRef,
    track_field: ChoiceField<TrackRef>,
    /// Car models with how many slots each gets, in grid order.
    cars: Vec<(String, usize)>,
    add_car: ChoiceField<String>,
    practice: u32,
    qualify: u32,
    /// Laps, or minutes for a timed race.
    race: u32,
    race_timed: bool,
    practice_field: NumberField,
    qualify_field: NumberField,
    race_field: NumberField,
    /// Seconds since midnight.
    time: u32,
    time_field: ChoiceField<u32>,
    weather: String,
    weather_field: ChoiceField<String>,
    errors: Vec<FieldError>,
    saving: bool,
}

impl Editor {
    fn new(
        url: String,
        id: PresetId,
        revision: Option<u64>,
        base: Preset,
        library: &Library,
        window: &mut Window,
        cx: &mut Context<HostScreen>,
    ) -> Self {
        let text = |value: &str,
                    placeholder: &'static str,
                    window: &mut Window,
                    cx: &mut Context<HostScreen>| {
            let value = value.to_owned();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let name = text(&base.name, "What the lobby shows", window, cx);
        let password = text(
            base.access.password.as_deref().unwrap_or_default(),
            "None: anyone may join",
            window,
            cx,
        );

        let mut tracks: Vec<(TrackRef, SharedString)> = library
            .tracks
            .iter()
            .filter(|track| track.owned)
            .flat_map(|track| {
                track.layouts.iter().map(move |layout| {
                    let label = match track.layouts.len() {
                        1 => track.name.clone(),
                        _ => format!("{} · {}", track.name, layout.name),
                    };
                    (
                        TrackRef {
                            id: track.id.clone(),
                            layout: layout.id.clone(),
                        },
                        SharedString::from(label),
                    )
                })
            })
            .collect();
        if !tracks.iter().any(|(track, _)| track == &base.track) {
            tracks.insert(
                0,
                (base.track.clone(), track_label(library, &base.track).into()),
            );
        }
        let track_field =
            ChoiceField::new(tracks, &base.track, window, cx, |this, track, _, cx| {
                if let Some(editor) = this.editor.as_mut() {
                    editor.track = track;
                }
                cx.notify();
            });

        let cars: Vec<(String, SharedString)> = library
            .cars
            .iter()
            .filter(|car| car.owned)
            .map(|car| (car.id.clone(), SharedString::from(car.name.clone())))
            .collect();
        let add_car = ChoiceField::new(
            cars,
            &String::new(),
            window,
            cx,
            |this, model, window, cx| {
                if let Some(editor) = this.editor.as_mut() {
                    match editor.cars.iter_mut().find(|(car, _)| *car == model) {
                        Some((_, slots)) => *slots += 1,
                        None => editor.cars.push((model, 1)),
                    }
                    editor.add_car.select(&String::new(), window, cx);
                }
                cx.notify();
            },
        );

        let mut weathers: Vec<(String, SharedString)> = library
            .weather
            .iter()
            .map(|weather| (weather.id.clone(), SharedString::from(weather.name.clone())))
            .collect();
        let weather = base
            .conditions
            .weather
            .first()
            .map(|weather| weather.graphics.clone())
            .unwrap_or_else(|| "3_clear".to_owned());
        if !weathers.iter().any(|(id, _)| *id == weather) {
            weathers.insert(0, (weather.clone(), weather.clone().into()));
        }
        let weather_field =
            ChoiceField::new(weathers, &weather, window, cx, |this, weather, _, cx| {
                if let Some(editor) = this.editor.as_mut() {
                    editor.weather = weather;
                }
                cx.notify();
            });

        let time = base.conditions.time_of_day;
        let mut times: Vec<(u32, SharedString)> = (0..24)
            .map(|hour| (hour * 3600, SharedString::from(format!("{hour:02}:00"))))
            .collect();
        if !time.is_multiple_of(3600) {
            let label = format!("{:02}:{:02}", time / 3600, time / 60 % 60);
            times.insert((time / 3600 + 1) as usize, (time, label.into()));
        }
        let time_field = ChoiceField::new(times, &time, window, cx, |this, time, _, cx| {
            if let Some(editor) = this.editor.as_mut() {
                editor.time = time;
            }
            cx.notify();
        });

        let sessions = &base.sessions;
        let practice = sessions.practice.map_or(0, |s| s.minutes.into());
        let qualify = sessions.qualify.map_or(0, |s| s.minutes.into());
        let (race, race_timed) = match sessions.race.map(|race| race.length) {
            None => (0, false),
            Some(RaceLength::Laps { laps }) => (laps.into(), false),
            Some(RaceLength::Minutes { minutes, .. }) => (minutes.into(), true),
        };
        let number = |value: u32,
                      window: &mut Window,
                      cx: &mut Context<HostScreen>,
                      set: fn(&mut Editor, u32)| {
            NumberField::new(
                value,
                window,
                cx,
                move |this: &mut HostScreen, value, _, cx| {
                    if let Some(editor) = this.editor.as_mut() {
                        set(editor, value);
                    }
                    cx.notify();
                },
            )
        };
        let practice_field = number(practice, window, cx, |e, v| e.practice = v);
        let qualify_field = number(qualify, window, cx, |e, v| e.qualify = v);
        let race_field = number(race, window, cx, |e, v| e.race = v);

        let mut cars: Vec<(String, usize)> = Vec::new();
        for entry in &base.entries {
            match cars.iter_mut().find(|(car, _)| *car == entry.model) {
                Some((_, slots)) => *slots += 1,
                None => cars.push((entry.model.clone(), 1)),
            }
        }

        Self {
            url,
            id,
            revision,
            name,
            password,
            track: base.track.clone(),
            track_field,
            cars,
            add_car,
            practice,
            qualify,
            race,
            race_timed,
            practice_field,
            qualify_field,
            race_field,
            time,
            time_field,
            weather,
            weather_field,
            errors: Vec::new(),
            saving: false,
            base,
        }
    }

    fn slots(&self) -> usize {
        self.cars.iter().map(|(_, slots)| slots).sum()
    }

    /// The preset as the editor has it: its controls over what it had.
    fn preset(&self, cx: &App) -> Preset {
        let base = &self.base;
        let mut preset = base.clone();
        preset.name = self.name.read(cx).value().trim().to_owned();
        preset.track = self.track.clone();

        // Slots of a car that stays keep their skins, drivers and the rest.
        preset.entries = Vec::new();
        for (model, slots) in &self.cars {
            let mut kept = base.entries.iter().filter(|e| &e.model == model).cloned();
            for _ in 0..*slots {
                preset
                    .entries
                    .push(kept.next().unwrap_or_else(|| fresh_entry(model)));
            }
        }
        let slots = preset.entries.len();

        let minutes = |value: u32| value.min(u32::from(u16::MAX)) as u16;
        let sessions = &mut preset.sessions;
        sessions.practice = (self.practice > 0).then(|| Session {
            minutes: minutes(self.practice),
            open: base.sessions.practice.is_none_or(|s| s.open),
        });
        sessions.qualify = (self.qualify > 0).then(|| Session {
            minutes: minutes(self.qualify),
            open: base.sessions.qualify.is_none_or(|s| s.open),
        });
        sessions.race = (self.race > 0).then(|| {
            let mut race = base.sessions.race.unwrap_or(Race {
                length: RaceLength::Laps { laps: 10 },
                join: RaceJoin::BeforeStart,
                wait_secs: 60,
                over_secs: 120,
                pit_window: None,
                reversed_grid: 0,
            });
            let value = minutes(self.race);
            race.length = match race.length {
                RaceLength::Minutes { extra_lap, .. } => RaceLength::Minutes {
                    minutes: value,
                    extra_lap,
                },
                RaceLength::Laps { .. } => RaceLength::Laps { laps: value },
            };
            race.reversed_grid = race.reversed_grid.min(slots.min(255) as u8);
            race
        });

        preset.conditions.time_of_day = self.time;
        if preset.conditions.weather.is_empty() {
            preset.conditions.weather = Preset::new("", preset.track.clone(), Vec::new())
                .conditions
                .weather;
        }
        preset.conditions.weather[0].graphics = self.weather.clone();

        let password = self.password.read(cx).value().trim().to_owned();
        preset.access.password = (!password.is_empty()).then_some(password);
        preset.access.max_clients = base
            .access
            .max_clients
            .filter(|&max| usize::from(max) <= slots);
        preset
    }
}

/// A track as the player's library names it, or by its folders.
fn track_label(library: &Library, track: &TrackRef) -> String {
    let found = library.tracks.iter().find(|t| t.id == track.id);
    match found {
        Some(found) => match found.layouts.iter().find(|l| l.id == track.layout) {
            Some(layout) if found.layouts.len() > 1 => format!("{} · {}", found.name, layout.name),
            Some(_) => found.name.clone(),
            None => format!("{} · {}", found.name, track.layout.as_deref().unwrap_or("")),
        },
        None => match &track.layout {
            Some(layout) => format!("{} · {layout}", track.id),
            None => track.id.clone(),
        },
    }
}

fn car_name(library: &Library, model: &str) -> String {
    library
        .cars
        .iter()
        .find(|car| car.id == model)
        .map_or_else(|| model.to_owned(), |car| car.name.clone())
}

/// How long ago `at`, seconds since 1970, was, in words.
fn ago(at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let secs = now.saturating_sub(at);
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => match secs / 86_400 {
            1 => "yesterday".to_owned(),
            days => format!("{days} days ago"),
        },
    }
}

fn server_state(state: &ServerState) -> String {
    match state {
        ServerState::Idle => "Not running".to_owned(),
        ServerState::Running { since } => {
            format!("Running · up since {}", ago(*since).replace(" ago", ""))
        }
        ServerState::Crashed {
            exit_code,
            restarts,
        } => {
            let code = exit_code.map_or(String::new(), |code| format!(" with {code}"));
            format!("Stopped{code}; restarting ({restarts})")
        }
    }
}

/// A text box the height of the other fields.
fn text_box(state: &Entity<InputState>, id: &'static str) -> impl IntoElement {
    Styled::h(Input::new(state).id(id).bg(rgb(color::BG)), px(36.))
}

/// A control under its label.
fn field(name: &str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(8.))
        .child(label(name))
        .child(control)
}

fn strip(text: SharedString, problem: bool) -> Div {
    div()
        .px(px(12.))
        .py(px(8.))
        .rounded(RADIUS)
        .bg(rgb(color::RAISED))
        .text_size(px(12.5))
        .text_color(rgb(if problem {
            color::WARN
        } else {
            color::TEXT_MUTED
        }))
        .child(text)
}

impl HostScreen {
    fn host_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("host-list")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(300.))
            .h_full()
            .border_r_1()
            .border_color(rgb(color::BORDER))
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(PAGE)
                    .pt(px(16.))
                    .pb(px(8.))
                    .child(display("Hosts", 14.).text_color(rgb(color::TEXT_MUTED)))
                    .when(!self.adding, |row| {
                        row.child(button("host-add-open", "Add a host").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.adding = true;
                                cx.notify();
                            },
                        )))
                    }),
            );
        for (index, host) in self.hosts.hosts.iter().enumerate() {
            let chosen = self.selected.as_ref() == Some(&host.url) && !self.adding;
            let dot = match self.views.get(&host.url) {
                Some(HostView { error: Some(_), .. }) => color::WARN,
                Some(HostView {
                    status: Some(status),
                    ..
                }) => match status.server {
                    ServerState::Running { .. } => color::GOOD,
                    ServerState::Crashed { .. } => color::WARN,
                    ServerState::Idle => color::TEXT_DIM,
                },
                _ => color::TEXT_DIM,
            };
            let url = host.url.clone();
            list = list.child(
                div()
                    .id(("host", index))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(PAGE)
                    .py(px(8.))
                    .cursor_pointer()
                    .map(|row| {
                        if chosen {
                            row.bg(rgb(color::ACCENT_WASH))
                        } else {
                            row.hover(|style| style.bg(rgb(color::SURFACE)))
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.select(url.clone(), cx)))
                    .child(div().size(px(8.)).flex_none().rounded_full().bg(rgb(dot)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .when(chosen, |name| name.font_weight(FontWeight::SEMIBOLD))
                                    .child(host.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(rgb(color::TEXT_DIM))
                                    .child(host.url.clone()),
                            ),
                    ),
            );
        }
        if self.adding {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .mx(PAGE)
                    .mt(px(12.))
                    .p(px(14.))
                    .rounded(RADIUS)
                    .border_1()
                    .border_color(rgb(color::BORDER))
                    .bg(rgb(color::SURFACE))
                    .child(display("Add a host", 16.))
                    .child(field("Name", text_box(&self.name_input, "host-name")))
                    .child(field(
                        "Address",
                        text_box(&self.address_input, "host-address"),
                    ))
                    .child(field("Token", text_box(&self.token_input, "host-token")))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(button("host-add", "Add host").light().on_click(
                                cx.listener(|this, _, window, cx| this.add_from_form(window, cx)),
                            ))
                            .when(!self.hosts.hosts.is_empty(), |row| {
                                row.child(button("host-add-cancel", "Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.adding = false;
                                        this.note = None;
                                        cx.notify();
                                    }),
                                ))
                            }),
                    ),
            );
        }
        list
    }

    fn intro(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .p(PAGE)
            .child(display("Run your own server", 27.))
            .child(div().max_w(px(520.)).text_center().child(muted(
                "A host is a machine running AssettoServer with Muretto's host agent beside it. \
                 Add one with its address and the token its owner gave you, then write presets for it \
                 and activate the one the server should run. Muretto uploads the cars and track a preset \
                 needs from your own game folder.",
            )))
    }

    fn host_page(&self, host: &KeptHost, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.views.get(&host.url);
        let library = self.game.read(cx).library.clone();
        let removing = self.confirm == Some(Confirm::RemoveHost(host.url.clone()));
        let header = div()
            .flex()
            .items_center()
            .gap(GAP)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(display(&host.name, 27.))
                    .child(muted(host.url.clone())),
            )
            .child(button("host-refresh", "Refresh").on_click(cx.listener({
                let url = host.url.clone();
                move |this, _, _, cx| this.refresh(&url, cx)
            })))
            .child(
                button(
                    "host-remove",
                    if removing {
                        "Really remove?"
                    } else {
                        "Remove host"
                    },
                )
                .on_click(cx.listener({
                    let url = host.url.clone();
                    move |this, _, _, cx| this.remove_host(&url, cx)
                })),
            );

        // The column grows with what it holds and the page around it
        // scrolls; as a flex child of the scrolling page it would be squeezed
        // to the window's height instead, cutting off the editor.
        let mut page = div().flex().flex_col().gap(GAP).p(PAGE).child(header);

        if let Some((text, problem)) = &self.note {
            page = page.child(strip(text.clone(), *problem));
        }
        if let Some(job) = &self.job {
            page = page.child(
                div()
                    .p(px(14.))
                    .rounded(RADIUS)
                    .border_1()
                    .border_color(rgb(color::BORDER))
                    .bg(rgb(color::SURFACE))
                    .child(job.strip()),
            );
        }
        if let Some(busy) = &self.busy {
            page = page.child(strip(busy.clone(), false));
        }
        if let Some(Confirm::Force(id, reason)) = &self.confirm {
            let id = *id;
            page =
                page.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(RADIUS)
                        .border_1()
                        .border_color(rgb(color::ACCENT_ACTIVE))
                        .bg(rgb(color::SURFACE))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(13.))
                                .child(reason.clone()),
                        )
                        .child(button("host-force", "Restart anyway").light().on_click(
                            cx.listener(move |this, _, _, cx| this.activate(id, true, cx)),
                        ))
                        .child(button("host-force-cancel", "Cancel").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.confirm = None;
                                cx.notify();
                            },
                        ))),
                );
        }

        match view {
            Some(HostView {
                error: Some(error), ..
            }) => page = page.child(strip(error.clone().into(), true)),
            Some(HostView {
                status: Some(status),
                ..
            }) => {
                let active = match &status.active {
                    Some(active) if active.changed_since => {
                        format!("{} (saved since)", active.name)
                    }
                    Some(active) => active.name.clone(),
                    None => "None".to_owned(),
                };
                page = page.child(stat_strip([
                    ("Server", server_state(&status.server).into()),
                    ("Preset", active.into()),
                    (
                        "Drivers",
                        status
                            .players
                            .map_or("–".to_owned(), |players| players.to_string())
                            .into(),
                    ),
                    ("Agent", status.agent_version.clone().into()),
                ]));
            }
            _ => page = page.child(muted("Asking the host…")),
        }

        page = page.child(self.track_models_switch(host, cx));
        page = match &self.editor {
            Some(editor) if editor.url == host.url => {
                page.child(self.editor_panel(editor, &library, cx))
            }
            _ => page.child(self.presets_panel(view, &library, cx)),
        };
        div()
            .id("host-page")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(page)
    }

    fn track_models_switch(&self, host: &KeptHost, cx: &mut Context<Self>) -> impl IntoElement {
        let url = host.url.clone();
        let screen = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(14.))
            .rounded(RADIUS)
            .border_1()
            .border_color(rgb(color::BORDER))
            .bg(rgb(color::SURFACE))
            .child(
                switch("host-track-models", host.track_models)
                    .caption("Check drivers' track models")
                    .on_toggle(move |on, _, cx| {
                        screen.update(cx, |screen, cx| screen.set_track_models(&url, on, cx))
                    }),
            )
            .child(muted(
                "Uploads the track's models with a preset, so the server turns away drivers whose track differs. \
                 They can run to hundreds of megabytes; off, only the few small files the server needs go up.",
            ))
    }

    fn presets_panel(
        &self,
        view: Option<&HostView>,
        library: &Library,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let presets = view.and_then(|view| view.presets.as_deref());
        let changed = view
            .and_then(|view| view.status.as_ref())
            .and_then(|status| status.active.as_ref())
            .filter(|active| active.changed_since)
            .map(|active| active.id);
        let mut body = div().flex().flex_col();
        match presets {
            None => {}
            Some([]) => {
                body = body.child(div().px(px(16.)).py(px(14.)).child(muted(
                    "No presets on this host yet. Write one: a track, the cars and the sessions.",
                )))
            }
            Some(presets) => {
                for (index, preset) in presets.iter().enumerate() {
                    body = body.child(self.preset_row(index, preset, changed, library, cx));
                }
            }
        }
        panel("Presets")
            .action(
                button("preset-new", "New preset")
                    .light()
                    .on_click(cx.listener(|this, _, window, cx| this.new_preset(window, cx))),
            )
            .child(body)
    }

    fn preset_row(
        &self,
        index: usize,
        preset: &PresetSummary,
        changed: Option<PresetId>,
        library: &Library,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = preset.id;
        let cars: Vec<String> = preset
            .cars
            .iter()
            .map(|car| car_name(library, car))
            .collect();
        let detail = format!(
            "{} · {} · {} slots",
            track_label(library, &preset.track),
            cars.join(", "),
            preset.slots
        );
        let deleting = self.confirm == Some(Confirm::DeletePreset(id));
        let activate = match (preset.active, changed == Some(id)) {
            (true, false) => None,
            (true, true) => Some("Apply changes"),
            (false, _) => Some("Activate"),
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .py(px(10.))
            .border_b_1()
            .border_color(rgb(color::BORDER))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(14.))
                                    .child(preset.name.clone()),
                            )
                            .when(preset.active, |row| row.child(badge("Running"))),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(color::TEXT_MUTED))
                            .child(detail),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(color::TEXT_DIM))
                            .font_family(font::BODY)
                            .child(format!(
                                "Saved by {} {}",
                                preset.updated_by,
                                ago(preset.updated_at)
                            )),
                    ),
            )
            .child(
                button(("preset-open", index), "Open").on_click(
                    cx.listener(move |this, _, window, cx| this.open_preset(id, window, cx)),
                ),
            )
            .when_some(activate, |row, text| {
                row.child(
                    button(("preset-activate", index), text)
                        .light()
                        .on_click(cx.listener(move |this, _, _, cx| this.activate(id, false, cx))),
                )
            })
            .child(
                button(
                    ("preset-delete", index),
                    if deleting { "Really delete?" } else { "Delete" },
                )
                .on_click(cx.listener(move |this, _, _, cx| this.delete_preset(id, cx))),
            )
    }

    fn editor_panel(
        &self,
        editor: &Editor,
        library: &Library,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let slots = editor.slots();
        let pit_boxes = library
            .tracks
            .iter()
            .find(|t| t.id == editor.track.id)
            .and_then(|t| t.layouts.iter().find(|l| l.id == editor.track.layout))
            .and_then(|layout| layout.pit_boxes);

        let mut cars = div().flex().flex_col().gap(px(6.));
        for (index, (model, count)) in editor.cars.iter().enumerate() {
            let installed = library.cars.iter().any(|car| &car.id == model);
            cars =
                cars.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(12.))
                        .py(px(6.))
                        .rounded(RADIUS)
                        .bg(rgb(color::BG))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.5))
                                .child(car_name(library, model)),
                        )
                        .when(!installed, |row| row.child(badge("Not installed")))
                        .child(
                            button(("preset-car-less", index), "−").on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(editor) = this.editor.as_mut()
                                        && editor.cars[index].1 > 1
                                    {
                                        editor.cars[index].1 -= 1;
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(mono(format!("{count} slots"), 13.).w(px(72.)).text_center())
                        .child(
                            button(("preset-car-more", index), "+").on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(editor) = this.editor.as_mut() {
                                        editor.cars[index].1 += 1;
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(button(("preset-car-remove", index), "Remove").on_click(
                            cx.listener(move |this, _, _, cx| {
                                if let Some(editor) = this.editor.as_mut() {
                                    editor.cars.remove(index);
                                }
                                cx.notify();
                            }),
                        )),
                );
        }
        let slots_note = match pit_boxes {
            Some(boxes) if slots as u32 > boxes => Some((
                format!("{slots} slots, but the track has {boxes} pit boxes."),
                true,
            )),
            Some(boxes) => Some((format!("{slots} slots of {boxes} pit boxes."), false)),
            None => None,
        };

        let race_name = if editor.race_timed {
            "Race minutes"
        } else {
            "Race laps"
        };
        let title = match editor.revision {
            Some(_) => "Edit preset",
            None => "New preset",
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .p(px(16.))
            .child(
                div()
                    .flex()
                    .gap(GAP)
                    .child(field("Name", text_box(&editor.name, "preset-name")))
                    .child(field("Track", editor.track_field.element())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label("Cars"))
                    .child(cars)
                    .child(div().w(px(320.)).child(editor.add_car.element_or("Add a car")))
                    .when_some(slots_note, |column, (text, problem)| {
                        column.child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(if problem {
                                    color::WARN
                                } else {
                                    color::TEXT_DIM
                                }))
                                .child(text),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(GAP)
                    .child(field(
                        "Practice minutes",
                        editor.practice_field.element("preset-practice", true),
                    ))
                    .child(field(
                        "Qualifying minutes",
                        editor.qualify_field.element("preset-qualify", true),
                    ))
                    .child(field(race_name, editor.race_field.element("preset-race", true))),
            )
            .child(muted("0 leaves a session out."))
            .child(
                div()
                    .flex()
                    .gap(GAP)
                    .child(field("Time of day", editor.time_field.element()))
                    .child(field("Weather", editor.weather_field.element()))
                    .child(field("Password", text_box(&editor.password, "preset-password"))),
            )
            .child(muted(
                "Rules, entry details such as skins and Steam ids, wind and AssettoServer's own options keep what the preset had.",
            ));
        if !editor.errors.is_empty() {
            body = body.child(div().flex().flex_col().gap(px(4.)).children(
                editor.errors.iter().map(|error| {
                    div()
                        .flex()
                        .gap(px(8.))
                        .text_size(px(12.5))
                        .child(mono(error.field.clone(), 12.).text_color(rgb(color::TEXT_DIM)))
                        .child(
                            div()
                                .text_color(rgb(color::WARN))
                                .child(error.message.clone()),
                        )
                }),
            ));
        }
        panel(title)
            .action(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        button("preset-cancel", "Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.close_editor(cx))),
                    )
                    .child(
                        button(
                            "preset-save",
                            if editor.saving { "Saving…" } else { "Save" },
                        )
                        .light()
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
            .child(body)
    }
}

impl Render for HostScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let right = match (self.adding, self.selected_host()) {
            (false, Some(host)) => self.host_page(&host, cx).into_any_element(),
            _ => div()
                .flex()
                .flex_col()
                .flex_1()
                .child(
                    div()
                        .px(PAGE)
                        .pt(PAGE)
                        .when_some(self.note.clone(), |column, (text, problem)| {
                            column.child(strip(text, problem))
                        }),
                )
                .child(self.intro())
                .into_any_element(),
        };
        div()
            .flex()
            .size_full()
            .child(self.host_list(cx))
            .child(right)
    }
}
