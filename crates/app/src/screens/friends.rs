//! Friends: the Online screen's groups. Everyone in a group sees which
//! server the others have open, can open the same one, and can follow a
//! friend so their choice becomes this player's. A group is joined once,
//! with a code, and rejoined whenever Muretto starts.

use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

use gpui_kit::{
    ClipboardItem, Context, Div, Entity, FontWeight, IntoElement, SharedString, Window,
    component::input::{Input, InputState},
    div,
    prelude::*,
    px, relative, rgb,
};
use muretto_core::{
    group::{Code, Event, Group, Groups, Member, Presence, Relay, Status, Viewed},
    online::{Found, Listed},
};

use super::online::OnlineScreen;
use crate::{
    theme::{PAGE, RADIUS, color, font},
    ui::{button, label, mono, muted},
};

/// How often news from the groups' threads is picked up.
const EVENTS_EVERY: Duration = Duration::from_millis(200);

/// The groups the player is in and how to reach the group server.
pub struct GroupSetup {
    pub kept: Groups,
    /// `None` when no group server is set.
    pub relay: Option<Arc<dyn Relay>>,
}

/// One group being taken part in.
struct Joined {
    code: Code,
    /// `None` while there is no server to join on.
    group: Option<Group>,
    status: Status,
    members: Vec<Member>,
}

pub struct Friends {
    kept: Groups,
    relay: Option<Arc<dyn Relay>>,
    joined: Vec<Joined>,
    /// What to call this player; groups are joined once it is known.
    name: Option<String>,
    /// The device of the friend whose server is followed.
    following: Option<String>,
    /// A server being asked about before it can be shown.
    jumping: Option<String>,
    /// The leave button of this group was pressed once.
    leaving: Option<Code>,
    code_input: Entity<InputState>,
    /// Something to tell the player, and whether it is a problem.
    note: Option<(SharedString, bool)>,
}

impl Friends {
    pub fn new<V: 'static>(setup: GroupSetup, window: &mut Window, cx: &mut Context<V>) -> Self {
        let code_input = cx.new(|cx| InputState::new(window, cx).placeholder("Paste a group code"));
        Self {
            kept: setup.kept,
            relay: setup.relay,
            joined: Vec::new(),
            name: None,
            following: None,
            jumping: None,
            leaving: None,
            code_input,
            note: None,
        }
    }

    /// Friends in all groups, each once.
    pub fn online(&self) -> usize {
        let mut devices: Vec<&str> = self
            .joined
            .iter()
            .flat_map(|joined| &joined.members)
            .map(|member| member.presence.device.as_str())
            .collect();
        devices.sort_unstable();
        devices.dedup();
        devices.len()
    }

    pub fn has_groups(&self) -> bool {
        !self.kept.groups.is_empty()
    }

    /// Addresses of the servers friends have open.
    pub fn viewed(&self) -> Vec<String> {
        self.members()
            .filter_map(|(_, member)| member.presence.viewing.as_ref())
            .map(Viewed::address)
            .collect()
    }

    fn members(&self) -> impl Iterator<Item = (&Joined, &Member)> {
        self.joined
            .iter()
            .flat_map(|joined| joined.members.iter().map(move |member| (joined, member)))
    }

    pub fn following(&self) -> Option<&str> {
        self.following.as_deref()
    }

    pub fn codes(&self) -> Vec<String> {
        self.kept
            .groups
            .iter()
            .map(|group| group.code.to_string())
            .collect()
    }

    /// The status of the group with `code`.
    pub fn status(&self, code: &str) -> Option<&Status> {
        self.joined
            .iter()
            .find(|joined| joined.code.to_string() == code)
            .map(|joined| &joined.status)
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_ref().map(|(text, _)| text.as_ref())
    }
}

impl OnlineScreen {
    /// Learns the player's name, then joins every kept group.
    pub(super) fn start_groups(&mut self, cx: &mut Context<Self>) {
        let source = self.source.clone();
        let asking = cx
            .background_executor()
            .spawn(async move { source.player() });
        cx.spawn(async move |this, cx| {
            let player = asking.await;
            this.update(cx, |this, cx| {
                this.friends.name = Some(player.map_or_else(|_| "Player".to_owned(), |p| p.name));
                let codes: Vec<Code> = this
                    .friends
                    .kept
                    .groups
                    .iter()
                    .map(|group| group.code.clone())
                    .collect();
                for code in codes {
                    this.take_part(code, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn presence(&self) -> Presence {
        Presence {
            device: self.friends.kept.device.clone(),
            name: self.friends.name.clone().unwrap_or_default(),
            viewing: self.selected().map(|listed| Viewed {
                ip: listed.server.ip.clone(),
                http_port: listed.server.http_port,
            }),
        }
    }

    /// Tells every group what this player has open now.
    pub(super) fn tell_groups(&self) {
        let presence = self.presence();
        for joined in &self.friends.joined {
            if let Some(group) = &joined.group {
                group.tell(presence.clone());
            }
        }
    }

    /// Connects to the group behind `code` and listens to it.
    fn take_part(&mut self, code: Code, cx: &mut Context<Self>) {
        if self.friends.name.is_none()
            || self.friends.joined.iter().any(|joined| joined.code == code)
        {
            return;
        }
        let Some(relay) = self.friends.relay.clone() else {
            self.friends.joined.push(Joined {
                code,
                group: None,
                status: Status::Offline(
                    "No group server is set. Set MURETTO_GROUP_SERVER, or \"server\" in groups.json."
                        .to_owned(),
                ),
                members: Vec::new(),
            });
            return;
        };
        let (sender, events) = mpsc::channel();
        let group = Group::join(relay, code.clone(), self.presence(), move |event| {
            let _ = sender.send(event);
        });
        self.friends.joined.push(Joined {
            code: code.clone(),
            group: Some(group),
            status: Status::Connecting,
            members: Vec::new(),
        });
        // What the group's thread reports is picked up a few times a second.
        // This ends when the group is left: its thread drops the sender.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(EVENTS_EVERY).await;
                let mut news = Vec::new();
                let left = loop {
                    match events.try_recv() {
                        Ok(event) => news.push(event),
                        Err(mpsc::TryRecvError::Empty) => break false,
                        Err(mpsc::TryRecvError::Disconnected) => break true,
                    }
                };
                if !news.is_empty() {
                    let alive = this.update(cx, |this, cx| {
                        for event in news {
                            this.group_event(&code, event, cx);
                        }
                        cx.notify();
                    });
                    if alive.is_err() {
                        break;
                    }
                }
                if left {
                    break;
                }
            }
        })
        .detach();
    }

    fn group_event(&mut self, code: &Code, event: Event, cx: &mut Context<Self>) {
        let Some(joined) = self
            .friends
            .joined
            .iter_mut()
            .find(|joined| &joined.code == code)
        else {
            return;
        };
        match event {
            Event::Status(status) => joined.status = status,
            Event::Members(members) => {
                joined.members = members;
                let names: Vec<String> = joined
                    .members
                    .iter()
                    .map(|member| member.presence.name.clone())
                    .collect();
                let _ = self.friends.kept.saw(code, &names);
                self.follow_along(cx);
            }
        }
        if matches!(self.tab(), super::online::Tab::Friends) {
            self.rescope();
        }
    }

    /// Opens what the followed friend has open, if that changed.
    fn follow_along(&mut self, cx: &mut Context<Self>) {
        let Some(device) = self.friends.following.clone() else {
            return;
        };
        let viewing = self
            .friends
            .members()
            .find(|(_, member)| member.presence.device == device)
            .and_then(|(_, member)| member.presence.viewing.clone());
        if let Some(viewing) = viewing {
            let current = self.selected().map(|listed| listed.server.address());
            if current.as_deref() != Some(viewing.address().as_str()) {
                self.open_server(&viewing, cx);
            }
        }
    }

    pub fn start_group(&mut self, cx: &mut Context<Self>) {
        let code = Code::random();
        self.friends.note = match self.friends.kept.add(code.clone()) {
            Ok(()) => Some((
                "Group started. Copy its code and send it to your friends.".into(),
                false,
            )),
            Err(error) => Some((format!("Could not keep the group: {error}").into(), true)),
        };
        self.take_part(code, cx);
        cx.notify();
    }

    /// Joins the group whose code is in the box.
    pub fn join_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.friends.code_input.read(cx).value().to_string();
        if self.join_code(&text, cx) {
            self.friends
                .code_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// Joins the group behind `text`; `false` when it is not a code.
    pub fn join_code(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(code) = Code::parse(text) else {
            self.friends.note = Some((
                "That is not a group code. A code is six groups of four letters and digits.".into(),
                true,
            ));
            cx.notify();
            return false;
        };
        if self
            .friends
            .kept
            .groups
            .iter()
            .any(|group| group.code == code)
        {
            self.friends.note = Some(("You are already in that group.".into(), false));
        } else {
            self.friends.note = match self.friends.kept.add(code.clone()) {
                Ok(()) => Some((
                    "Joined. Muretto rejoins this group whenever it starts.".into(),
                    false,
                )),
                Err(error) => Some((format!("Could not keep the group: {error}").into(), true)),
            };
            self.take_part(code, cx);
        }
        cx.notify();
        true
    }

    /// Asks once, then leaves the group and forgets its code.
    pub fn leave_group(&mut self, code: &Code, cx: &mut Context<Self>) {
        if self.friends.leaving.as_ref() != Some(code) {
            self.friends.leaving = Some(code.clone());
            cx.notify();
            return;
        }
        self.friends.leaving = None;
        self.friends.joined.retain(|joined| &joined.code != code);
        self.friends.note = self
            .friends
            .kept
            .remove(code)
            .err()
            .map(|error| (format!("Could not forget the group: {error}").into(), true));
        let still_there = self.friends.following.as_ref().is_some_and(|device| {
            self.friends
                .members()
                .any(|(_, m)| &m.presence.device == device)
        });
        if !still_there {
            self.friends.following = None;
        }
        cx.notify();
    }

    pub fn copy_code(&mut self, code: &Code, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
        self.friends.note = Some(("Code copied.".into(), false));
        cx.notify();
    }

    /// Opens the server a friend has open.
    pub fn jump_to(&mut self, device: &str, cx: &mut Context<Self>) {
        let viewing = self
            .friends
            .members()
            .find(|(_, member)| member.presence.device == device)
            .and_then(|(_, member)| member.presence.viewing.clone());
        if self.friends.following.as_deref() != Some(device) {
            self.friends.following = None;
        }
        if let Some(viewing) = viewing {
            self.open_server(&viewing, cx);
        }
        cx.notify();
    }

    /// Follows a friend, or stops following them.
    pub fn toggle_follow(&mut self, device: &str, cx: &mut Context<Self>) {
        if self.friends.following.as_deref() == Some(device) {
            self.friends.following = None;
        } else {
            self.friends.following = Some(device.to_owned());
            self.follow_along(cx);
        }
        cx.notify();
    }

    /// The player picked a server themselves: following ends.
    pub(super) fn stop_following(&mut self) {
        self.friends.following = None;
    }

    /// Selects the server at `viewed`, asking it about itself first when the
    /// list does not have it.
    fn open_server(&mut self, viewed: &Viewed, cx: &mut Context<Self>) {
        let address = viewed.address();
        if let Some(index) = self.index_of(&address) {
            self.friends.jumping = None;
            self.select(index, cx);
            return;
        }
        if self.friends.jumping.as_deref() == Some(address.as_str()) {
            return;
        }
        self.friends.jumping = Some(address.clone());
        let (source, ip, port) = (self.source.clone(), viewed.ip.clone(), viewed.http_port);
        let asking = cx
            .background_executor()
            .spawn(async move { source.info(&ip, port) });
        cx.spawn(async move |this, cx| {
            let answer = asking.await;
            this.update(cx, |this, cx| {
                if this.friends.jumping.as_deref() != Some(address.as_str()) {
                    return;
                }
                match answer {
                    Ok(mut server) => {
                        server.found = Found::Direct;
                        this.add_server(server, cx);
                    }
                    Err(error) => {
                        this.friends.jumping = None;
                        this.friends.note =
                            Some((format!("{address} did not answer: {error}").into(), true));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Called with each new listing: shows the server a jump was waiting for.
    pub(super) fn listed_jump(&mut self, cx: &mut Context<Self>) {
        let Some(address) = self.friends.jumping.clone() else {
            return;
        };
        if let Some(index) = self.index_of(&address) {
            self.friends.jumping = None;
            self.select(index, cx);
        }
    }

    /// What a friend's server is called here: its name in the list, or
    /// its address while it is not in it.
    fn server_name(&self, viewed: &Viewed) -> (String, Option<&Listed>) {
        let address = viewed.address();
        match self.index_of(&address) {
            Some(index) => {
                let listed = &self.listing().servers[index];
                (listed.server.name.clone(), Some(listed))
            }
            None => (address, None),
        }
    }

    /// The Friends tab, where the server list is on the others.
    pub(super) fn friends_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let friends = &self.friends;
        let join_row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .w(px(340.))
                    .flex_none()
                    .child(Input::new(&friends.code_input)),
            )
            .child(
                button("group-join", "Join group")
                    .on_click(cx.listener(|this, _, window, cx| this.join_group(window, cx))),
            )
            .child(div().flex_1())
            .child(
                button("group-start", "Start a group")
                    .light()
                    .on_click(cx.listener(|this, _, _, cx| this.start_group(cx))),
            );

        let you = match self.presence().viewing {
            Some(viewing) => format!(
                "Your groups see that you have {} open.",
                self.server_name(&viewing).0
            ),
            None => "Your groups see that you have no server open.".to_owned(),
        };

        let mut column = div()
            .id("friends")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(14.))
            .px(PAGE)
            .py(px(14.))
            .overflow_y_scroll()
            .child(join_row);
        if let Some((text, problem)) = &friends.note {
            column = column.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(RADIUS)
                    .bg(rgb(color::RAISED))
                    .text_size(px(12.5))
                    .text_color(rgb(if *problem {
                        color::WARN
                    } else {
                        color::TEXT_MUTED
                    }))
                    .child(text.clone()),
            );
        }
        if friends.kept.groups.is_empty() {
            return column.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("No groups yet"))
                    .child(div().max_w(px(460.)).text_center().child(muted(
                        "Start a group and send its code to your friends, or paste a code a friend sent you. \
                         Everyone in a group sees which server the others have open, and Muretto rejoins your groups whenever it starts.",
                    ))),
            );
        }
        column = column.child(muted(you));
        for (index, group) in friends.kept.groups.iter().enumerate() {
            let joined = friends
                .joined
                .iter()
                .find(|joined| joined.code == group.code);
            column = column.child(self.group_card(index, &group.code, &group.known, joined, cx));
        }
        column
    }

    fn group_card(
        &self,
        index: usize,
        code: &Code,
        known: &[String],
        joined: Option<&Joined>,
        cx: &mut Context<Self>,
    ) -> Div {
        let (dot, status): (u32, SharedString) = match joined.map(|joined| &joined.status) {
            Some(Status::Connected) => (color::GOOD, "Connected".into()),
            Some(Status::Offline(why)) => (color::WARN, format!("Offline. {why}").into()),
            Some(Status::Connecting) | None => (color::TEXT_DIM, "Connecting…".into()),
        };
        let members = joined
            .map(|joined| joined.members.as_slice())
            .unwrap_or_default();
        let leaving = self.friends.leaving.as_ref() == Some(code);

        let header = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(12.))
            .border_b_1()
            .border_color(rgb(color::BORDER))
            .child(label("Group"))
            .child(mono(code.to_string(), 13.))
            .child(div().size(px(8.)).flex_none().rounded_full().bg(rgb(dot)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .text_color(rgb(color::TEXT_MUTED))
                    .truncate()
                    .child(status),
            )
            .child(
                button(("group-copy", index), "Copy code").on_click(cx.listener({
                    let code = code.clone();
                    move |this, _, _, cx| this.copy_code(&code, cx)
                })),
            )
            .child(
                button(
                    ("group-leave", index),
                    if leaving { "Really leave?" } else { "Leave" },
                )
                .on_click(cx.listener({
                    let code = code.clone();
                    move |this, _, _, cx| this.leave_group(&code, cx)
                })),
            );

        let body = if members.is_empty() {
            let text = if known.is_empty() {
                "Nobody else is here yet. Send them the code.".to_owned()
            } else {
                format!(
                    "Nobody else is here right now. Seen here before: {}.",
                    known.join(", ")
                )
            };
            div().px(px(16.)).py(px(14.)).child(muted(text))
        } else {
            div().flex().flex_col().children(
                members
                    .iter()
                    .enumerate()
                    .map(|(row, member)| self.member_row(index * 100 + row, member, cx)),
            )
        };

        div()
            .flex()
            .flex_col()
            .flex_none()
            .rounded(RADIUS)
            .border_1()
            .border_color(rgb(color::BORDER))
            .bg(rgb(color::SURFACE))
            .child(header)
            .child(body)
    }

    fn member_row(&self, id: usize, member: &Member, cx: &mut Context<Self>) -> Div {
        let presence = &member.presence;
        let following = self.friends.following.as_deref() == Some(presence.device.as_str());
        let name = if presence.name.trim().is_empty() {
            "Someone".to_owned()
        } else {
            presence.name.clone()
        };
        let current = self.selected().map(|listed| listed.server.address());
        let (looking, detail, same) = match &presence.viewing {
            Some(viewing) => {
                let (server_name, listed) = self.server_name(viewing);
                let detail = match listed {
                    Some(listed) => format!(
                        "{} · {} of {} drivers",
                        self.listing().track_name(&listed.track_id),
                        listed.server.clients,
                        listed.server.capacity
                    ),
                    None if self.friends.jumping.as_deref() == Some(viewing.address().as_str()) => {
                        "Asking the server…".to_owned()
                    }
                    None => "Not in your list".to_owned(),
                };
                (
                    Some(server_name),
                    detail,
                    current.as_deref() == Some(viewing.address().as_str()),
                )
            }
            None => (None, String::new(), false),
        };
        let initial = name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        let device = presence.device.clone();

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
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(30.))
                    .rounded_full()
                    .bg(rgb(color::CHIP))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.))
                    .child(initial),
            )
            .child(
                div()
                    .w(px(150.))
                    .flex_none()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .child(name),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .map(|cell| match &looking {
                        Some(server) => cell
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .line_height(relative(1.3))
                                    .child(server.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(color::TEXT_MUTED))
                                    .font_family(font::BODY)
                                    .child(detail.clone()),
                            ),
                        None => cell.child(muted("No server open")),
                    }),
            )
            .map(|row| match (&looking, same) {
                (Some(_), true) => row.child(
                    div()
                        .flex_none()
                        .text_size(px(12.5))
                        .text_color(rgb(color::GOOD))
                        .child("Same as you"),
                ),
                (Some(_), false) => {
                    row.child(button(("friend-jump", id), "Open").on_click(cx.listener({
                        let device = device.clone();
                        move |this, _, _, cx| this.jump_to(&device, cx)
                    })))
                }
                (None, _) => row,
            })
            .child({
                let follow = button(
                    ("friend-follow", id),
                    if following { "Following" } else { "Follow" },
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_follow(&device, cx)));
                if following { follow.light() } else { follow }
            })
    }
}
