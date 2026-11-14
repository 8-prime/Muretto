//! Links the DLL without a build timestamp, so the same source always gives
//! the same bytes and `cargo make shim-check` can tell whether the copy
//! checked in at `crates/core/assets/dbghelp.dll` is current.

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let gnu = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    if windows && gnu {
        println!("cargo:rustc-link-arg=-Wl,--no-insert-timestamp");
    }
}
