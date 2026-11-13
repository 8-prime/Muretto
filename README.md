# Muretto

A launcher and content manager for Assetto Corsa, for Linux and Windows.
Written in Rust on [GPUI](https://gpui.rs) through [GPUI Kit](https://gpui-kit.com).

The interface follows `docs/design/AC Launcher — UI Design.pdf`.

## Layout

| Path | What it is |
| --- | --- |
| `crates/core` | Everything about the game that is not interface: finding the install, reading cars, tracks and weather, writing `race.ini`, starting the simulator. No UI dependencies; compiles in seconds. |
| `crates/app` | The window. `theme.rs` holds colours and type, `ui/` the controls, `shell.rs` the frame, `screens/` one file per screen. |

## Running

```sh
cargo run                                   # the app
cargo test --workspace                      # core logic and simulated-input UI tests
cargo run -p muretto-core --example scan    # print what was found on this machine
```

The game is located through Steam's library folders. Set `MURETTO_AC_ROOT`
to the game folder to override that.

To compare the interface with the design without opening a window, render it
offscreen to a PNG. Element ids after the path are clicked first:

```sh
cargo run -p muretto --features shot --bin shot -- drive.png
cargo run -p muretto --features shot --bin shot -- cars.png change-car
```

## Fonts and icons

Barlow Condensed, Inter and JetBrains Mono (SIL Open Font License) and a few
Lucide icons (ISC) are compiled in from `crates/app/assets`, with their
licences beside them.
