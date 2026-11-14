# Muretto

A launcher and content manager for Assetto Corsa, for Linux and Windows.
Written in Rust on [GPUI](https://gpui.rs) through [GPUI Kit](https://gpui-kit.com).

The interface follows `docs/design/AC Launcher — UI Design.pdf`.

## Layout

| Path | What it is |
| --- | --- |
| `crates/core` | Everything about the game that is not interface: finding the install, reading cars, tracks and weather, writing `race.ini`, starting the simulator, and installing packages such as Custom Shaders Patch. No UI dependencies; compiles in seconds. |
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

## What Muretto writes into the game folder

Installing a package (so far: Custom Shaders Patch and Pure, under Settings) unpacks it
into `.muretto/staging` inside the game folder, checks it, then moves the
files into place. A receipt of every file is kept in `.muretto/packages`, so
an update clears out what the old version left behind and a removal takes
exactly what was added. Game files a mod overwrites are set aside in
`.muretto/backups` and put back when the mod is removed.

Mods Muretto cannot download itself are described by a recipe in
`crates/core/src/packages/recipes.rs`: where the player gets it, how to
recognise its archive, and what using it means. Getting one opens its page in
the browser and a dialog in Muretto that takes the downloaded zip by drop or
file choice. Add a mod by adding a recipe.

To try an install against the real download without touching the game:

```sh
cargo run -p muretto-core --example csp -- /tmp/fake-game
```

## Fonts and icons

Barlow Condensed, Inter and JetBrains Mono (SIL Open Font License) and a few
Lucide icons (ISC) are compiled in from `crates/app/assets`, with their
licences beside them.
