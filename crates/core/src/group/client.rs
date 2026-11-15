//! Being in a group: a thread that holds the connection to the group's room,
//! tells the room what this player is looking at, and reports who else is
//! there. It reconnects by itself and ends when the [`Group`] is dropped.

use std::{
    collections::BTreeMap,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use muretto_group_protocol::{ClientMessage, ServerMessage};

use super::{Code, Keys, Presence, Relay};

/// The least time between two states sent; changes in between are merged.
/// Keeps well under the server's limit however fast servers are clicked.
const SEND_EVERY: Duration = Duration::from_millis(250);
/// How long to wait before reconnecting, at first and at most.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MOST: Duration = Duration::from_secs(30);
/// How often the thread looks for news from the player while it waits.
const TICK: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected,
    /// Why the connection is down; the thread tries again by itself.
    Offline(String),
}

/// Someone else in the group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The server's id for the member's connection.
    pub id: u64,
    pub presence: Presence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Status(Status),
    /// Everyone else in the group now, in the order they joined.
    Members(Vec<Member>),
}

/// Membership in one group, while it lasts.
pub struct Group {
    code: Code,
    updates: mpsc::Sender<Presence>,
}

impl Group {
    /// Joins the group behind `code` through `relay`, saying `presence`, and
    /// reports what happens to `events`, from another thread.
    pub fn join(
        relay: Arc<dyn Relay>,
        code: Code,
        presence: Presence,
        events: impl Fn(Event) + Send + 'static,
    ) -> Self {
        let (updates, news) = mpsc::channel();
        let keys = code.keys();
        std::thread::Builder::new()
            .name("muretto-group".to_owned())
            .spawn(move || run(relay.as_ref(), &keys, presence, &news, &events))
            .expect("a thread starts");
        Self { code, updates }
    }

    pub fn code(&self) -> &Code {
        &self.code
    }

    /// Tells the group this player's new presence.
    pub fn tell(&self, presence: Presence) {
        let _ = self.updates.send(presence);
    }
}

/// Why a connection ended.
enum Ended {
    /// The [`Group`] was dropped: stop.
    Left,
    Lost(String),
}

fn run(
    relay: &dyn Relay,
    keys: &Keys,
    mut presence: Presence,
    news: &mpsc::Receiver<Presence>,
    events: &dyn Fn(Event),
) {
    let mut retry = RETRY_FIRST;
    loop {
        events(Event::Status(Status::Connecting));
        let reason = match relay.open(&keys.room) {
            Ok(pipe) => {
                retry = RETRY_FIRST;
                match connected(pipe, keys, &mut presence, news, events) {
                    Ended::Left => return,
                    Ended::Lost(reason) => {
                        events(Event::Members(Vec::new()));
                        reason
                    }
                }
            }
            Err(error) => error.to_string(),
        };
        events(Event::Status(Status::Offline(reason)));

        // Wait before trying again, keeping up with the player meanwhile.
        let until = Instant::now() + retry;
        retry = (retry * 2).min(RETRY_MOST);
        while Instant::now() < until {
            match news.recv_timeout(TICK) {
                Ok(newer) => presence = newer,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn connected(
    mut pipe: Box<dyn super::Pipe>,
    keys: &Keys,
    presence: &mut Presence,
    news: &mpsc::Receiver<Presence>,
    events: &dyn Fn(Event),
) -> Ended {
    let mut members: BTreeMap<u64, Presence> = BTreeMap::new();
    let mut unsent = true;
    let mut sent = Instant::now() - SEND_EVERY;
    loop {
        loop {
            match news.try_recv() {
                Ok(newer) => {
                    unsent |= newer != *presence;
                    *presence = newer;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ended::Left,
            }
        }
        if unsent && sent.elapsed() >= SEND_EVERY {
            let data = keys.seal(&presence.to_bytes());
            if let Err(error) = pipe.send(&ClientMessage::State { data }) {
                return Ended::Lost(error.to_string());
            }
            unsent = false;
            sent = Instant::now();
        }

        let message = match pipe.recv(TICK) {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(error) => return Ended::Lost(error.to_string()),
        };
        let open = |data: &str| {
            keys.open(data)
                .and_then(|bytes| Presence::from_bytes(&bytes))
        };
        match message {
            ServerMessage::Welcome { members: there, .. } => {
                members = there
                    .iter()
                    .filter_map(|member| Some((member.id, open(&member.data)?)))
                    .collect();
                events(Event::Status(Status::Connected));
            }
            // A state that does not open was not sealed with this group's
            // key; it is ignored.
            ServerMessage::State { id, data } => match open(&data) {
                Some(presence) => {
                    members.insert(id, presence);
                }
                None => continue,
            },
            ServerMessage::Left { id } => {
                if members.remove(&id).is_none() {
                    continue;
                }
            }
            ServerMessage::Error { message } => return Ended::Lost(message),
        }
        events(Event::Members(everyone_else(&members, &presence.device)));
    }
}

/// The members to show: not this install, whose last connection may linger
/// on the server for a while, and each other install once, by its newest
/// connection.
fn everyone_else(members: &BTreeMap<u64, Presence>, device: &str) -> Vec<Member> {
    let mut newest: BTreeMap<&str, u64> = BTreeMap::new();
    for (&id, presence) in members {
        newest.insert(&presence.device, id);
    }
    members
        .iter()
        .filter(|(id, presence)| {
            presence.device != device && newest[presence.device.as_str()] == **id
        })
        .map(|(&id, presence)| Member {
            id,
            presence: presence.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, atomic::Ordering};

    use super::*;
    use crate::group::{Viewed, relay::Memory};

    /// What one member has heard so far.
    #[derive(Clone, Default)]
    struct Heard(Arc<Mutex<Vec<Event>>>);

    impl Heard {
        fn sink(&self) -> impl Fn(Event) + Send + 'static {
            let heard = self.0.clone();
            move |event| heard.lock().unwrap().push(event)
        }

        fn last_members(&self) -> Option<Vec<Member>> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|event| match event {
                    Event::Members(members) => Some(members.clone()),
                    _ => None,
                })
        }

        fn last_status(&self) -> Option<Status> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|event| match event {
                    Event::Status(status) => Some(status.clone()),
                    _ => None,
                })
        }

        /// Waits until `check` holds for what was heard.
        fn until(&self, what: &str, check: impl Fn(&Heard) -> bool) {
            let until = Instant::now() + Duration::from_secs(5);
            while !check(self) {
                assert!(Instant::now() < until, "timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    /// `viewing` is the server's http port, on 10.0.0.1.
    fn presence(device: &str, name: &str, viewing: Option<u16>) -> Presence {
        Presence {
            device: device.into(),
            name: name.into(),
            viewing: viewing.map(|http_port| Viewed {
                ip: "10.0.0.1".into(),
                http_port,
            }),
        }
    }

    fn names(members: &[Member]) -> Vec<(String, Option<String>)> {
        members
            .iter()
            .map(|member| {
                (
                    member.presence.name.clone(),
                    member.presence.viewing.as_ref().map(Viewed::address),
                )
            })
            .collect()
    }

    #[test]
    fn friends_with_the_same_code_see_each_other_and_strangers_do_not() {
        let relay: Arc<dyn Relay> = Arc::new(Memory::default());
        let code = Code::random();
        let (alex_heard, sam_heard, stranger_heard) =
            (Heard::default(), Heard::default(), Heard::default());
        let alex = Group::join(
            relay.clone(),
            code.clone(),
            presence("a", "Alex", Some(8081)),
            alex_heard.sink(),
        );
        let _sam = Group::join(
            relay.clone(),
            code.clone(),
            presence("s", "Sam", None),
            sam_heard.sink(),
        );
        let _stranger = Group::join(
            relay.clone(),
            Code::random(),
            presence("x", "Stranger", None),
            stranger_heard.sink(),
        );

        sam_heard.until("Alex's server", |heard| {
            heard.last_members().is_some_and(|members| {
                names(&members) == [("Alex".into(), Some("10.0.0.1:8081".into()))]
            })
        });
        alex_heard.until("Sam", |heard| {
            heard
                .last_members()
                .is_some_and(|members| names(&members) == [("Sam".into(), None)])
        });
        assert_eq!(alex_heard.last_status(), Some(Status::Connected));

        alex.tell(presence("a", "Alex", Some(8082)));
        sam_heard.until("the new server", |heard| {
            heard.last_members().is_some_and(|members| {
                names(&members) == [("Alex".into(), Some("10.0.0.1:8082".into()))]
            })
        });

        drop(alex);
        sam_heard.until("Alex leaving", |heard| {
            heard
                .last_members()
                .is_some_and(|members| members.is_empty())
        });
        assert!(
            stranger_heard
                .last_members()
                .is_none_or(|members| members.is_empty()),
            "another group hears nothing"
        );
    }

    #[test]
    fn a_dropped_connection_comes_back_by_itself() {
        let memory = Memory::default();
        let relay: Arc<dyn Relay> = Arc::new(memory.clone());
        memory.down.store(true, Ordering::SeqCst);
        let code = Code::random();
        let heard = Heard::default();
        let _alex = Group::join(
            relay.clone(),
            code.clone(),
            presence("a", "Alex", None),
            heard.sink(),
        );
        heard.until("the first failure", |heard| {
            matches!(heard.last_status(), Some(Status::Offline(_)))
        });
        memory.down.store(false, Ordering::SeqCst);
        heard.until("the retry", |heard| {
            heard.last_status() == Some(Status::Connected)
        });
        let sam_heard = Heard::default();
        let _sam = Group::join(relay, code, presence("s", "Sam", None), sam_heard.sink());
        sam_heard.until("Alex", |heard| {
            heard
                .last_members()
                .is_some_and(|members| members.len() == 1)
        });
    }

    #[test]
    fn the_same_install_shows_once_and_never_as_someone_else() {
        let mut members = BTreeMap::new();
        members.insert(1, presence("a", "Alex (old)", None));
        members.insert(2, presence("me", "Me", None));
        members.insert(3, presence("a", "Alex", None));
        members.insert(4, presence("s", "Sam", None));
        let shown: Vec<String> = everyone_else(&members, "me")
            .into_iter()
            .map(|member| member.presence.name)
            .collect();
        assert_eq!(shown, ["Alex", "Sam"]);
    }
}
