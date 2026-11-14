//! Reads the official lobby with this machine's Steam account and the
//! installed game's content, and prints what the Online screen would offer:
//!
//! `cargo run -p muretto-core --release --example online [track id]`

use std::time::Instant;

use muretto_core::{
    Install, Library,
    online::{Facet, Filter, Listing, Official, Source as _, fetch_slots},
    steam,
};

fn main() {
    let player = steam::current_user().expect("a Steam account on this machine");
    let install = Install::discover().expect("Assetto Corsa is installed");
    let library = Library::scan(&install);

    let started = Instant::now();
    let lobby = Official.servers(&player).expect("the lobby answers");
    let servers = lobby.servers;
    println!("{} servers in {:?}", servers.len(), started.elapsed());
    if let Some((age, why)) = lobby.stale {
        println!("from a copy {age:?} old: {why}");
    }
    let started = Instant::now();
    let listing = Listing::new(servers, &library);
    println!("listed in {:?}", started.elapsed());

    let mut filter = Filter {
        not_empty: true,
        ..Filter::default()
    };
    if let Some(track) = std::env::args().nth(1) {
        filter.toggle(Facet::Track, &track);
    }
    let started = Instant::now();
    let matching = listing.matching(&filter);
    let facets: Vec<_> = Facet::ALL
        .iter()
        .map(|&facet| (facet, listing.facet(facet, &filter)))
        .collect();
    println!(
        "{} not empty; filtered and counted in {:?}",
        matching.len(),
        started.elapsed()
    );
    for (facet, values) in facets {
        println!("{}: {} values", facet.label(), values.len());
        for value in values.iter().take(6) {
            println!(
                "  {:>4}  {}{}",
                value.count,
                value.name,
                match value.installed {
                    Some(true) => "  (installed)",
                    _ => "",
                }
            );
        }
    }

    for &index in matching.iter().take(5) {
        let listed = &listing.servers[index];
        let server = &listed.server;
        println!(
            "{:>3}/{:<3} {:<60.60} {} · {} cars ({} missing)",
            server.clients,
            server.capacity,
            server.name,
            listing.track_name(&listed.track_id),
            server.cars.len(),
            listed.missing_cars(),
        );
    }
    if let Some(&first) = matching.first() {
        match fetch_slots(&listing.servers[first].server, &player) {
            Ok(slots) => println!(
                "first server: {} slots, {} taken",
                slots.len(),
                slots.iter().filter(|s| s.connected).count()
            ),
            Err(error) => println!("first server: {error}"),
        }
    }
}
