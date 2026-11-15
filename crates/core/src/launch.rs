//! Starting the game: write the session where the game looks for it, then run
//! the simulator executable directly, skipping the stock launcher.

use std::{
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::SystemTime,
};

use crate::{
    Error, Install, Library, Result, SessionConfig,
    install::{Runtime, STEAM_APP_ID},
    io_err,
    packages::csp,
};

/// Writes `race.ini` and `assists.ini` for `config` into the game's settings
/// folder, creating the folder from the game's defaults if the game has never
/// been started.
pub fn write_session(install: &Install, library: &Library, config: &SessionConfig) -> Result<()> {
    prepare(install)?;
    let cfg = install.cfg_dir();
    write(&cfg.join("race.ini"), &config.race_ini(library).to_string())?;
    write(
        &cfg.join("assists.ini"),
        &config.assists.to_ini().to_string(),
    )
}

/// Readies the user's folder for a start, as the stock launcher would: the
/// default settings, the logs folder, and under Proton what the patch needs.
fn prepare(install: &Install) -> Result<()> {
    seed_defaults(&install.root.join("cfg"), &install.cfg_dir())?;
    // Without it the game keeps no log, and the patch's crash reporter
    // crashes while writing its report, so a failed start leaves no trace.
    let logs = install.logs_dir();
    std::fs::create_dir_all(&logs).map_err(io_err(&logs))?;
    if matches!(install.runtime, Runtime::Proton { .. })
        && install.root.join("dwrite.dll").is_file()
    {
        csp::provide_fonts(install)?;
    }
    Ok(())
}

/// The stock launcher copies the default settings into the user's folder on
/// first start. The simulator will not run without them, so do the same for
/// any that are missing; existing files are never touched.
pub(crate) fn seed_defaults(defaults: &Path, cfg: &Path) -> Result<()> {
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

pub(crate) fn write(path: &Path, contents: &str) -> Result<()> {
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

/// Wine prefers its own DLLs to ones beside the executable. Custom Shaders
/// Patch loads as the game's `dwrite.dll`, and Muretto's fix for it under
/// Proton is a `dbghelp.dll` there too (see [`csp::apply_proton_fix`]): tell
/// Wine to use the game's copies, keeping any overrides the player already set.
fn dll_overrides(install: &Install, existing: Option<String>) -> Option<String> {
    let mut ours = Vec::new();
    if install.root.join("dwrite.dll").is_file() {
        ours.push("dwrite=n,b");
    }
    let shim = std::fs::read(install.root.join(csp::SHIM_FILE)).ok();
    if shim.is_some_and(|bytes| csp::is_shim(&bytes)) {
        ours.push("dbghelp=n,b");
    }
    if ours.is_empty() {
        return None;
    }
    let ours = ours.join(";");
    Some(match existing.filter(|value| !value.is_empty()) {
        Some(existing) => format!("{existing};{ours}"),
        None => ours,
    })
}

/// Writes `race.ini` from `join` and starts the game, which connects to the
/// server instead of starting a session of its own.
pub fn join(install: &Install, join: &crate::online::Join) -> Result<Child> {
    prepare(install)?;
    let cfg = install.cfg_dir();
    write(&cfg.join("race.ini"), &join.race_ini().to_string())?;
    command(install)?
        .spawn()
        .map_err(|e| Error::Launch(format!("Could not start the game: {e}")))
}

/// Writes the session and starts the game.
pub fn start(install: &Install, library: &Library, config: &SessionConfig) -> Result<Child> {
    write_session(install, library, config)?;
    let mut command = command(install)?;
    command
        .spawn()
        .map_err(|e| Error::Launch(format!("Could not start the game: {e}")))
}

/// How a game started at `started` ended: an error when it crashed or
/// stopped with a failure, saying where to look.
pub fn outcome(install: &Install, started: SystemTime, status: ExitStatus) -> Result<()> {
    let logs = install.logs_dir();
    let crash_report = std::fs::read_dir(&logs)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("crash_")
                && name.ends_with(".report")
                && entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .is_ok_and(|modified| modified >= started)
        });
    if crash_report {
        return Err(Error::Launch(format!(
            "The game crashed. Custom Shaders Patch left a crash report in {}.",
            logs.display()
        )));
    }
    if status.success() {
        return Ok(());
    }
    let code = status
        .code()
        .map_or_else(|| "a signal".to_owned(), |code| format!("code {code}"));
    Err(Error::Launch(format!(
        "The game stopped unexpectedly ({code}). Its log is in {}.",
        logs.display()
    )))
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

        // A dbghelp.dll of unknown origin is left to Wine's judgement; the
        // Proton fix is preferred to Wine's own.
        std::fs::write(root.join("dbghelp.dll"), "MZ other").unwrap();
        assert_eq!(dll_overrides(&install, None).as_deref(), Some("dwrite=n,b"));
        std::fs::remove_file(root.join("dbghelp.dll")).unwrap();
        csp::apply_proton_fix(&install).unwrap();
        assert_eq!(
            dll_overrides(&install, None).as_deref(),
            Some("dwrite=n,b;dbghelp=n,b")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn a_start_under_proton_gives_the_patch_its_fonts_and_a_logs_folder() {
        let dir = temp_dir("prepare");
        let root = dir.join("steamapps/common/assettocorsa");
        let fonts = root.join("content/fonts");
        std::fs::create_dir_all(&fonts).unwrap();
        std::fs::write(fonts.join("segoeui.ttf"), "regular").unwrap();
        std::fs::write(fonts.join("segoeuii.ttf"), "italic").unwrap();
        let install = Install::at(root.clone(), Some(dir.join("steam")));

        // No patch: the game folder is left as it is.
        prepare(&install).unwrap();
        assert!(install.logs_dir().is_dir());
        assert!(!fonts.join("system").exists());

        std::fs::write(root.join("dwrite.dll"), "").unwrap();
        std::fs::create_dir_all(fonts.join("system")).unwrap();
        std::fs::write(fonts.join("system/verdana.ttf"), "the player's").unwrap();
        prepare(&install).unwrap();
        let read = |name: &str| std::fs::read_to_string(fonts.join("system").join(name)).unwrap();
        assert_eq!(read("segoeui.ttf"), "regular");
        assert_eq!(read("verdana.ttf"), "the player's");
        assert_eq!(read("verdanai.ttf"), "italic");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_crash_report_written_since_the_start_means_the_game_crashed() {
        use std::os::unix::process::ExitStatusExt;
        let dir = temp_dir("outcome");
        let install = Install::at(dir.join("steamapps/common/assettocorsa"), None);
        let logs = install.logs_dir();
        std::fs::create_dir_all(&logs).unwrap();
        let clean = ExitStatus::from_raw(0);
        let failed = ExitStatus::from_raw(5 << 8);

        std::fs::write(logs.join("crash_old.report"), "").unwrap();
        let started = SystemTime::now() + std::time::Duration::from_secs(1);
        assert!(outcome(&install, started, clean).is_ok());
        assert!(
            matches!(outcome(&install, started, failed), Err(Error::Launch(m)) if m.contains("code 5"))
        );

        let started = SystemTime::now() - std::time::Duration::from_secs(1);
        assert!(
            matches!(outcome(&install, started, clean), Err(Error::Launch(m)) if m.contains("crash report"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
