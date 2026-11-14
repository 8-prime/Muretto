//! How far away servers are, and which ones run on this network.
//!
//! acServer and AssettoServer both answer a four-byte datagram on their UDP
//! port, `200` as a little-endian integer, with three bytes: `200` again and
//! their HTTP port. The round trip is the ping; sent as a broadcast, the same
//! datagram finds servers on the local network.

use std::{
    collections::{HashMap, HashSet},
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const PING: [u8; 4] = [200, 0, 0, 0];

/// Ports Content Manager looks for local servers on; servers set up for a
/// LAN use one of these.
pub const LAN_PORTS: &[u16] = &[
    9456, 9457, 9458, 9556, 9600, 9601, 9602, 9603, 9604, 9605, 9606, 9607, 9608, 9609, 9610, 9611,
    9612, 9700,
];

/// The HTTP port from an answer, if `reply` is one.
fn answer(reply: &[u8]) -> Option<u16> {
    match reply {
        [200, low, high] => Some(u16::from_le_bytes([*low, *high])),
        _ => None,
    }
}

/// Pings every target once and calls `answered` with each reply as it
/// arrives. Returns when every target has answered, or `patience` after the
/// last ping went out.
pub fn ping(
    targets: &[SocketAddr],
    patience: Duration,
    mut answered: impl FnMut(SocketAddr, Duration),
) -> std::io::Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_read_timeout(Some(Duration::from_millis(40)))?;
    let sender = socket.try_clone()?;
    let sent: Arc<Mutex<HashMap<SocketAddr, Instant>>> = Arc::default();

    // Send from another thread so that answers are timed as they arrive,
    // not after the last ping has gone out.
    let sending = thread::spawn({
        let sent = sent.clone();
        let targets = targets.to_vec();
        move || {
            for (index, target) in targets.into_iter().enumerate() {
                sent.lock().unwrap().insert(target, Instant::now());
                let _ = sender.send_to(&PING, target);
                // Thousands at once overflow the send buffer; pace them.
                if index % 64 == 63 {
                    thread::sleep(Duration::from_millis(4));
                }
            }
        }
    });

    let mut buffer = [0; 16];
    let mut all_sent: Option<Instant> = None;
    loop {
        match socket.recv_from(&mut buffer) {
            Ok((length, from)) => {
                if answer(&buffer[..length]).is_some()
                    && let Some(at) = sent.lock().unwrap().remove(&from)
                {
                    answered(from, at.elapsed());
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            // Windows reports an unreachable port of an earlier ping on the
            // next read. Nothing to do but keep listening.
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
        if all_sent.is_none() && sending.is_finished() {
            all_sent = Some(Instant::now());
        }
        if let Some(at) = all_sent
            && (at.elapsed() >= patience || sent.lock().unwrap().is_empty())
        {
            break;
        }
    }
    let _ = sending.join();
    Ok(())
}

/// A server that answered a broadcast: where it is, its HTTP port, and how
/// long it took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub address: SocketAddr,
    pub http_port: u16,
    pub ping: Duration,
}

/// Broadcasts a ping to every port in `ports` and collects the servers that
/// answer within `patience`.
pub fn broadcast(ports: &[u16], patience: Duration) -> std::io::Result<Vec<Found>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(Duration::from_millis(40)))?;
    let started = Instant::now();
    for &port in ports {
        let _ = socket.send_to(&PING, (Ipv4Addr::BROADCAST, port));
    }
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut buffer = [0; 16];
    while started.elapsed() < patience {
        match socket.recv_from(&mut buffer) {
            Ok((length, from)) => {
                if let Some(http_port) = answer(&buffer[..length])
                    && seen.insert(from)
                {
                    found.push(Found {
                        address: from,
                        http_port,
                        ping: started.elapsed(),
                    });
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server on the loopback interface that answers pings like acServer.
    fn server(http_port: u16) -> SocketAddr {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = socket.local_addr().unwrap();
        thread::spawn(move || {
            let mut buffer = [0; 16];
            while let Ok((length, from)) = socket.recv_from(&mut buffer) {
                if buffer[..length] == PING {
                    let [low, high] = http_port.to_le_bytes();
                    socket.send_to(&[200, low, high], from).unwrap();
                }
            }
        });
        address
    }

    #[test]
    fn servers_that_answer_are_timed_and_the_rest_waited_for_briefly() {
        let up = server(8081);
        // A bound socket that never answers.
        let silent = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let down = silent.local_addr().unwrap();

        let mut answers = Vec::new();
        let started = Instant::now();
        ping(&[up, down], Duration::from_millis(300), |from, took| {
            answers.push((from, took))
        })
        .unwrap();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].0, up);
        assert!(answers[0].1 < Duration::from_millis(300));
        assert!(started.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn answers_are_three_bytes_with_the_http_port() {
        assert_eq!(answer(&[200, 0xc4, 0x1f]), Some(8132));
        assert_eq!(answer(&[200, 0, 0, 0]), None);
        assert_eq!(answer(&[1, 2, 3]), None);
    }
}
