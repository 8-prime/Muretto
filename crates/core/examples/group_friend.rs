//! Joins a group as a pretend friend, to try groups without a second
//! machine. It reports who else is there, and each line typed on standard
//! input is a server to have open (`ip:http-port`, or empty for none):
//!
//! `cargo run -p muretto-core --example group_friend -- ws://127.0.0.1:3100 XXXX-XXXX-XXXX-XXXX-XXXX-XXXX Alex [ip:http-port]`

use std::{io::BufRead as _, sync::Arc};

use muretto_core::group::{Code, Event, Group, Presence, Viewed, WebSocketRelay};

fn viewed(text: &str) -> Option<Viewed> {
    let (ip, port) = text.trim().rsplit_once(':')?;
    Some(Viewed {
        ip: ip.to_owned(),
        http_port: port.parse().ok()?,
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let server = args.next().expect("the group server, ws:// or wss://");
    let code = Code::parse(&args.next().expect("a group code")).expect("a valid group code");
    let name = args.next().unwrap_or_else(|| "Friend".to_owned());
    let mut presence = Presence {
        device: format!("example-{}", std::process::id()),
        name,
        viewing: args.next().as_deref().and_then(viewed),
    };

    let group = Group::join(
        Arc::new(WebSocketRelay::new(server)),
        code,
        presence.clone(),
        |event| match event {
            Event::Status(status) => println!("status: {status:?}"),
            Event::Members(members) => {
                println!("members:");
                for member in members {
                    let viewing = member.presence.viewing.as_ref().map(Viewed::address);
                    println!("  {} viewing {viewing:?}", member.presence.name);
                }
            }
        },
    );
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        presence.viewing = viewed(&line);
        group.tell(presence.clone());
    }
    // Standard input closed: stay until stopped.
    loop {
        std::thread::park();
    }
}
