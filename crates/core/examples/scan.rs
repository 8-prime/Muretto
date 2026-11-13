//! Prints what Muretto finds on this machine: `cargo run -p muretto-core --example scan`

use muretto_core::{Install, Library, SessionConfig};

fn main() {
    let install = match Install::discover() {
        Ok(install) => install,
        Err(e) => return eprintln!("{e}"),
    };
    println!("{install:#?}");
    println!("shaders patch: {:?}", install.shaders_patch());

    let started = std::time::Instant::now();
    let library = Library::scan(&install);
    println!(
        "{} cars ({} owned), {} tracks, {} weather in {:?}",
        library.cars.len(),
        library.cars.iter().filter(|c| c.owned).count(),
        library.tracks.len(),
        library.weather.len(),
        started.elapsed()
    );
    for track in &library.tracks {
        let layouts: Vec<_> = track.layouts.iter().map(|l| l.name.as_str()).collect();
        println!(
            "  {:<24} {:<28} {:?} {:?}",
            track.id, track.name, track.origin, layouts
        );
    }
    if let Some(config) = SessionConfig::first_run(&library) {
        println!(
            "\n{}\n\n{}",
            config.summary(&library),
            config.race_ini(&library)
        );
    }
}
