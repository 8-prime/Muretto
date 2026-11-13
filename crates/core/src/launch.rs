//! Starting the game: write the session where the game looks for it, then run
//! the simulator executable directly, skipping the stock launcher.

use std::{
    path::Path,
    process::{Child, Command, Stdio},
};

use crate::{
    Error, Install, Library, Result, SessionConfig,
    install::{Runtime, STEAM_APP_ID},
    io_err,
};

/// Writes `race.ini` and `assists.ini` for `config` into the game's settings
/// folder, creating the folder from the game's defaults if the game has never
/// been started.
pub fn write_session(install: &Install, library: &Library, config: &SessionConfig) -> Result<()> {
    let cfg = install.cfg_dir();
    seed_defaults(&install.root.join("cfg"), &cfg)?;
    write(&cfg.join("race.ini"), &config.race_ini(library).to_string())?;
    write(
        &cfg.join("assists.ini"),
        &config.assists.to_ini().to_string(),
    )
}

/// The stock launcher copies the default settings into the user's folder on
/// first start. The simulator will not run without them, so do the same for
/// any that are missing; existing files are never touched.
fn seed_defaults(defaults: &Path, cfg: &Path) -> Result<()> {
    std::fs::create_dir_all(cfg).map_err(io_err(cfg))?;
    let Ok(entries) = std::fs::read_dir(defaults) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let target = cfg.join(entry.file_name());
        if source.is_file() && !target.exists() {
            std::fs::copy(&source, &target).map_err(io_err(&target))?;
        }
    }
    Ok(())
}

fn write(path: &Path, contents: &str) -> Result<()> {
    // The game expects Windows line endings in files it later rewrites.
    std::fs::write(path, contents.replace('\n', "\r\n")).map_err(io_err(path))
}

/// The command that starts the simulator with whatever session is currently
/// written. Steam must be running: the game checks ownership through it.
pub fn command(install: &Install) -> Result<Command> {
    let executable = install.executable();
    if !executable.is_file() {
        return Err(Error::Launch(format!(
            "{} is missing. Verify the game files in Steam.",
            executable.display()
        )));
    }
    let app_id = STEAM_APP_ID.to_string();
    let mut command = match &install.runtime {
        Runtime::Native => Command::new(&executable),
        Runtime::Proton {
            steam_root,
            compat_data,
            proton,
        } => {
            let proton = proton.as_ref().ok_or_else(|| {
                Error::Launch(
                    "The game has no Proton prefix yet. Start Assetto Corsa once from Steam, close it, then try again."
                        .to_owned(),
                )
            })?;
            let mut command = Command::new(proton.join("proton"));
            command
                .arg("run")
                .arg(&executable)
                .env("STEAM_COMPAT_DATA_PATH", compat_data)
                .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", steam_root);
            if let Some(overrides) = dll_overrides(install, std::env::var("WINEDLLOVERRIDES").ok())
            {
                command.env("WINEDLLOVERRIDES", overrides);
            }
            command
        }
    };
    command
        .current_dir(&install.root)
        .env("SteamAppId", &app_id)
        .env("SteamGameId", &app_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

/// Wine prefers its own `dwrite.dll` to one beside the executable, which is
/// how Custom Shaders Patch loads. When the patch is installed, tell Wine to
/// use the game's copy, keeping any overrides the player already set.
fn dll_overrides(install: &Install, existing: Option<String>) -> Option<String> {
    if !install.root.join("dwrite.dll").is_file() {
        return None;
    }
    Some(match existing.filter(|value| !value.is_empty()) {
        Some(existing) => format!("{existing};dwrite=n,b"),
        None => "dwrite=n,b".to_owned(),
    })
}

/// Writes the session and starts the game.
pub fn start(install: &Install, library: &Library, config: &SessionConfig) -> Result<Child> {
    write_session(install, library, config)?;
    let mut command = command(install)?;
    command
        .spawn()
        .map_err(|e| Error::Launch(format!("Could not start the game: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("muretto-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_settings_are_seeded_and_existing_ones_kept() {
        let dir = temp_dir("seed");
        let defaults = dir.join("defaults");
        let cfg = dir.join("docs/cfg");
        std::fs::create_dir_all(&defaults).unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(defaults.join("video.ini"), "default").unwrap();
        std::fs::write(defaults.join("controls.ini"), "default").unwrap();
        std::fs::write(cfg.join("controls.ini"), "mine").unwrap();

        seed_defaults(&defaults, &cfg).unwrap();

        assert_eq!(
            std::fs::read_to_string(cfg.join("video.ini")).unwrap(),
            "default"
        );
        assert_eq!(
            std::fs::read_to_string(cfg.join("controls.ini")).unwrap(),
            "mine"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn proton_command_runs_the_simulator_inside_the_prefix() {
        let dir = temp_dir("command");
        let root = dir.join("steamapps/common/assettocorsa");
        let proton = dir.join("proton-build");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("acs.exe"), "").unwrap();
        let mut install = Install::at(root.clone(), Some(dir.join("steam")));

        // No prefix yet: explain instead of failing obscurely.
        assert!(matches!(command(&install), Err(Error::Launch(m)) if m.contains("Proton prefix")));

        if let Runtime::Proton { proton: slot, .. } = &mut install.runtime {
            *slot = Some(proton.clone());
        }
        let command = command(&install).unwrap();
        assert_eq!(command.get_program(), proton.join("proton"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["run".as_ref(), root.join("acs.exe").as_os_str()]);
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
        let env: Vec<_> = command.get_envs().map(|(k, _)| k.to_owned()).collect();
        assert!(env.contains(&"STEAM_COMPAT_DATA_PATH".into()));
        assert!(env.contains(&"SteamAppId".into()));
        assert!(
            !env.contains(&"WINEDLLOVERRIDES".into()),
            "no patch, no override"
        );

        // With the patch's loader in the game folder, Wine is told to use it.
        std::fs::write(root.join("dwrite.dll"), "").unwrap();
        assert_eq!(dll_overrides(&install, None).as_deref(), Some("dwrite=n,b"));
        assert_eq!(
            dll_overrides(&install, Some("dxgi=b".to_owned())).as_deref(),
            Some("dxgi=b;dwrite=n,b")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
