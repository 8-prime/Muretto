//! What Muretto and its group server say to each other, and how the server
//! keeps its rooms.
//!
//! Friends who share a group code meet in a room on the server. Each member
//! sends its latest state, an opaque encrypted blob; the server keeps the
//! latest one per member, hands all of them to whoever joins, and passes
//! every change on to the rest of the room. It never sees the code, the key,
//! or what is inside a state.
//!
//! Messages are JSON text frames over a WebSocket at `/v1/{room}`, where
//! `room` is 64 lowercase hex characters derived from the group code.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

/// Where the WebSocket for a room lives, before the room id.
pub const PATH_PREFIX: &str = "/v1/";
/// How many members a room holds at once.
pub const MAX_MEMBERS: usize = 16;
/// The longest state a member may send, in characters of its base64 text.
pub const MAX_STATE: usize = 1024;
/// The length of a room id, in hex characters.
pub const ROOM_LEN: usize = 64;
/// The longest text frame either side needs to accept.
pub const MAX_FRAME: usize = 4 * 1024 + MAX_MEMBERS * (MAX_STATE + 64);

/// From a member to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    /// The member's state now; replaces what it sent before.
    State { data: String },
}

/// From the server to a member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The first message after joining: the id the server knows this
    /// connection by, and the latest state of every member that has sent one.
    Welcome { you: u64, members: Vec<MemberState> },
    /// Another member's state changed, or a member sent its first.
    State { id: u64, data: String },
    /// A member left the room.
    Left { id: u64 },
    /// The server refused something; it closes the connection after this.
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberState {
    pub id: u64,
    pub data: String,
}

impl ClientMessage {
    pub fn to_text(&self) -> String {
        serde_json::to_string(self).expect("messages serialise")
    }
}

impl ServerMessage {
    pub fn to_text(&self) -> String {
        serde_json::to_string(self).expect("messages serialise")
    }
}

/// Whether `room` is a room id: 64 lowercase hex characters.
pub fn valid_room(room: &str) -> bool {
    room.len() == ROOM_LEN
        && room
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Why the server turned a member away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    BadRoom,
    Full,
    TooLarge,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::BadRoom => "That is not a group room.",
            Refusal::Full => "This group is full.",
            Refusal::TooLarge => "That state is too large.",
        })
    }
}

/// Where the server puts messages for one member. `push` reports `false`
/// when the member is gone or not keeping up; the room then drops it.
pub trait Outbox {
    fn push(&self, message: ServerMessage) -> bool;
}

struct Member<O> {
    outbox: O,
    data: Option<String>,
}

/// Every room with someone in it. The server holds one of these behind a
/// lock; each member's connection owns nothing but the receiving end of its
/// outbox, so dropping a member from a room ends its connection.
pub struct Rooms<O> {
    rooms: HashMap<String, BTreeMap<u64, Member<O>>>,
    next_id: u64,
}

impl<O> Default for Rooms<O> {
    fn default() -> Self {
        Self {
            rooms: HashMap::new(),
            next_id: 1,
        }
    }
}

impl<O: Outbox> Rooms<O> {
    /// Puts a new member in `room` and sends it the welcome.
    pub fn join(&mut self, room: &str, outbox: O) -> Result<u64, Refusal> {
        if !valid_room(room) {
            return Err(Refusal::BadRoom);
        }
        let members = self.rooms.entry(room.to_owned()).or_default();
        if members.len() >= MAX_MEMBERS {
            return Err(Refusal::Full);
        }
        let id = self.next_id;
        self.next_id += 1;
        let welcome = ServerMessage::Welcome {
            you: id,
            members: members
                .iter()
                .filter_map(|(&id, member)| {
                    Some(MemberState {
                        id,
                        data: member.data.clone()?,
                    })
                })
                .collect(),
        };
        if outbox.push(welcome) {
            members.insert(id, Member { outbox, data: None });
        } else if members.is_empty() {
            self.rooms.remove(room);
        }
        Ok(id)
    }

    /// Keeps the member's new state and passes it on to the others.
    pub fn set_state(&mut self, room: &str, id: u64, data: String) -> Result<(), Refusal> {
        if data.is_empty() || data.len() > MAX_STATE {
            return Err(Refusal::TooLarge);
        }
        let Some(members) = self.rooms.get_mut(room) else {
            return Ok(());
        };
        let Some(member) = members.get_mut(&id) else {
            return Ok(());
        };
        member.data = Some(data.clone());
        let gone: Vec<u64> = members
            .iter()
            .filter(|&(&other, _)| other != id)
            .filter(|(_, member)| {
                !member.outbox.push(ServerMessage::State {
                    id,
                    data: data.clone(),
                })
            })
            .map(|(&other, _)| other)
            .collect();
        for other in gone {
            self.leave(room, other);
        }
        Ok(())
    }

    /// Takes the member out of the room and tells the rest. A room nobody
    /// is in is forgotten.
    pub fn leave(&mut self, room: &str, id: u64) {
        let mut leaving = vec![id];
        while let Some(id) = leaving.pop() {
            let Some(members) = self.rooms.get_mut(room) else {
                return;
            };
            if members.remove(&id).is_none() {
                continue;
            }
            for (&other, member) in members.iter() {
                if !member.outbox.push(ServerMessage::Left { id }) && !leaving.contains(&other) {
                    leaving.push(other);
                }
            }
            if members.is_empty() {
                self.rooms.remove(room);
            }
        }
    }

    /// How many members are in rooms altogether.
    pub fn members(&self) -> usize {
        self.rooms.values().map(BTreeMap::len).sum()
    }

    pub fn rooms(&self) -> usize {
        self.rooms.len()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    use super::*;

    #[derive(Clone, Default)]
    struct Inbox {
        got: Rc<RefCell<Vec<ServerMessage>>>,
        closed: Rc<Cell<bool>>,
    }

    impl Outbox for Inbox {
        fn push(&self, message: ServerMessage) -> bool {
            if self.closed.get() {
                return false;
            }
            self.got.borrow_mut().push(message);
            true
        }
    }

    impl Inbox {
        fn take(&self) -> Vec<ServerMessage> {
            std::mem::take(&mut self.got.borrow_mut())
        }
    }

    fn room(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    #[test]
    fn a_late_joiner_gets_everyone_s_latest_state_and_later_changes_reach_the_rest() {
        let mut rooms = Rooms::default();
        let (a, b, c) = (Inbox::default(), Inbox::default(), Inbox::default());
        let ida = rooms.join(&room(1), a.clone()).unwrap();
        let idb = rooms.join(&room(1), b.clone()).unwrap();
        assert_eq!(
            a.take(),
            [ServerMessage::Welcome {
                you: ida,
                members: vec![]
            }]
        );
        rooms.set_state(&room(1), ida, "one".into()).unwrap();
        rooms.set_state(&room(1), ida, "two".into()).unwrap();
        assert!(a.take().is_empty(), "a member does not hear itself");
        b.take();

        let idc = rooms.join(&room(1), c.clone()).unwrap();
        // b never sent a state, so it is not listed yet.
        assert_eq!(
            c.take(),
            [ServerMessage::Welcome {
                you: idc,
                members: vec![MemberState {
                    id: ida,
                    data: "two".into()
                }]
            }]
        );
        rooms.set_state(&room(1), idb, "b".into()).unwrap();
        let expected = ServerMessage::State {
            id: idb,
            data: "b".into(),
        };
        assert_eq!(a.take(), std::slice::from_ref(&expected));
        assert_eq!(c.take(), [expected]);

        rooms.leave(&room(1), idb);
        assert_eq!(a.take(), [ServerMessage::Left { id: idb }]);
        assert_eq!(c.take(), [ServerMessage::Left { id: idb }]);
        rooms.leave(&room(1), ida);
        rooms.leave(&room(1), idc);
        assert_eq!(rooms.rooms(), 0, "an empty room is forgotten");
    }

    #[test]
    fn rooms_are_apart_full_rooms_refuse_and_bad_input_is_refused() {
        let mut rooms = Rooms::default();
        let (a, b) = (Inbox::default(), Inbox::default());
        let ida = rooms.join(&room(1), a.clone()).unwrap();
        rooms.join(&room(2), b.clone()).unwrap();
        rooms.set_state(&room(1), ida, "x".into()).unwrap();
        let heard = b.take();
        assert!(
            matches!(heard.as_slice(), [ServerMessage::Welcome { members, .. }] if members.is_empty()),
            "another room hears nothing: {heard:?}"
        );

        for _ in 1..MAX_MEMBERS {
            rooms.join(&room(1), Inbox::default()).unwrap();
        }
        assert_eq!(rooms.join(&room(1), Inbox::default()), Err(Refusal::Full));
        assert_eq!(
            rooms.join("not-a-room", Inbox::default()),
            Err(Refusal::BadRoom)
        );
        assert_eq!(
            rooms.join(&room(0xab).to_uppercase(), Inbox::default()),
            Err(Refusal::BadRoom)
        );
        assert_eq!(
            rooms.set_state(&room(1), ida, "x".repeat(MAX_STATE + 1)),
            Err(Refusal::TooLarge)
        );
        assert_eq!(
            rooms.set_state(&room(1), ida, String::new()),
            Err(Refusal::TooLarge)
        );
    }

    #[test]
    fn a_member_that_stops_taking_messages_is_dropped_and_the_rest_are_told() {
        let mut rooms = Rooms::default();
        let (a, b, c) = (Inbox::default(), Inbox::default(), Inbox::default());
        let ida = rooms.join(&room(1), a.clone()).unwrap();
        let idb = rooms.join(&room(1), b.clone()).unwrap();
        rooms.join(&room(1), c.clone()).unwrap();
        a.take();
        c.take();
        b.closed.set(true);
        rooms.set_state(&room(1), ida, "x".into()).unwrap();
        assert_eq!(c.take().last(), Some(&ServerMessage::Left { id: idb }));
        assert_eq!(rooms.members(), 2);
    }

    #[test]
    fn messages_read_as_documented() {
        assert_eq!(
            ClientMessage::State {
                data: "QQ==".into()
            }
            .to_text(),
            r#"{"type":"state","data":"QQ=="}"#
        );
        assert!(
            serde_json::from_str::<ClientMessage>(r#"{"type":"state","data":"a","x":1}"#).is_err()
        );
        assert_eq!(
            ServerMessage::Welcome {
                you: 3,
                members: vec![MemberState {
                    id: 1,
                    data: "a".into()
                }]
            }
            .to_text(),
            r#"{"type":"welcome","you":3,"members":[{"id":1,"data":"a"}]}"#
        );
        assert_eq!(
            ServerMessage::Left { id: 2 }.to_text(),
            r#"{"type":"left","id":2}"#
        );
    }
}
