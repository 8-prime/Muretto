//! Microsoft's shader compiler, for Custom Shaders Patch under Proton.
//!
//! The patch compiles shaders with `d3dcompiler_47.dll`. Proton's is Wine's
//! own, which fails on the patch's includes. Weather FX's linear colour space
//! then misses its final pass, and the game shows an almost black picture.
//! Microsoft does not offer the DLL as a download of its own. Like winetricks,
//! Muretto takes it from an old Firefox installer, which ships it. Both the
//! installer and the DLL are checked against known hashes.

use sha2::{Digest, Sha256};

use super::{Progress, Stage, download, downloads_dir};
use crate::{Error, Install, Result, io_err};

pub(crate) const FILE: &str = "d3dcompiler_47.dll";
const INSTALLER_URL: &str = "https://download-installer.cdn.mozilla.net/pub/firefox/releases/62.0.3/win64/ach/Firefox%20Setup%2062.0.3.exe";
const INSTALLER_SIZE: u64 = 40_176_024;
const INSTALLER_SHA256: &str = "721977f36c008af2b637aedd3f1b529f3cfed6feb10f68ebe17469acb1934986";
/// The DLL's path inside the installer's archive.
const ENTRY: &str = "core/d3dcompiler_47.dll";
/// Version 10.0.15063.674, 64-bit.
const DLL_SHA256: &str = "0427b16ffb7d2f6f2d611cc9f0520f42c70bcbfbdc2fc0d47e8735fd1c79a308";
/// Where the 7z archive of a self-extracting installer starts.
const SEVEN_ZIP_SIGNATURE: &[u8] = b"7z\xbc\xaf\x27\x1c";

/// What is in the game folder under the compiler's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compiler {
    Absent,
    /// The copy Muretto installs.
    Installed,
    /// Some other file. It is left alone, and the game still uses it.
    Other,
}

pub fn compiler(install: &Install) -> Compiler {
    match std::fs::read(install.root.join(FILE)) {
        Err(_) => Compiler::Absent,
        Ok(bytes) if sha256(&bytes) == DLL_SHA256 => Compiler::Installed,
        Ok(_) => Compiler::Other,
    }
}

/// Downloads the installer, takes the compiler out of it and puts it beside
/// the game. Does nothing when a compiler is already there.
pub(crate) fn install(install: &Install, progress: &Progress) -> Result<()> {
    if compiler(install) != Compiler::Absent {
        return Ok(());
    }
    let installer = downloads_dir(&install.root).join("firefox-62.0.3-setup.exe");
    download(INSTALLER_URL, &installer, Some(INSTALLER_SIZE), progress)?;
    let result = (|| {
        progress.begin(Stage::Unpacking, None);
        let data = std::fs::read(&installer).map_err(io_err(&installer))?;
        if sha256(&data) != INSTALLER_SHA256 {
            return Err(Error::Archive(
                "The downloaded Firefox installer is not the expected file.".to_owned(),
            ));
        }
        let dll = extract(&data)?;
        if sha256(&dll) != DLL_SHA256 {
            return Err(Error::Archive(
                "The shader compiler in the Firefox installer is not the expected file.".to_owned(),
            ));
        }
        progress.begin(Stage::Installing, None);
        let staging = install.root.join(".muretto-d3dcompiler_47.dll");
        let target = install.root.join(FILE);
        std::fs::write(&staging, &dll).map_err(io_err(&staging))?;
        std::fs::rename(&staging, &target).map_err(io_err(&target))
    })();
    let _ = std::fs::remove_file(&installer);
    result
}

/// Takes Muretto's copy out again; any other file stays.
pub(crate) fn remove(install: &Install) -> Result<()> {
    if compiler(install) == Compiler::Installed {
        let path = install.root.join(FILE);
        std::fs::remove_file(&path).map_err(io_err(&path))?;
    }
    Ok(())
}

/// The compiler's bytes, from the 7z archive appended to a self-extracting
/// installer.
fn extract(installer: &[u8]) -> Result<Vec<u8>> {
    let unreadable =
        |e: &dyn std::fmt::Display| Error::Archive(format!("The installer could not be read: {e}"));
    // The signature may also occur by chance in the stub before the archive.
    let mut last_error = None;
    for start in (0..installer.len()).filter(|&at| installer[at..].starts_with(SEVEN_ZIP_SIGNATURE))
    {
        let archive = std::io::Cursor::new(&installer[start..]);
        let mut reader =
            match sevenz_rust2::ArchiveReader::new(archive, sevenz_rust2::Password::empty()) {
                Ok(reader) => reader,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
        let mut found = None;
        reader
            .for_each_entries_with_decision(|entry, data| {
                use sevenz_rust2::EntryDecision;
                if entry.name() != ENTRY {
                    return Ok(EntryDecision::Continue);
                }
                let mut bytes = Vec::new();
                data.read_to_end(&mut bytes)?;
                found = Some(bytes);
                Ok(EntryDecision::Stop)
            })
            .map_err(|e| unreadable(&e))?;
        return found
            .ok_or_else(|| Error::Archive(format!("The installer does not contain {ENTRY}.")));
    }
    Err(match last_error {
        Some(error) => unreadable(&error),
        None => unreadable(&"no 7z archive found"),
    })
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiler_is_taken_from_the_archive_behind_an_installer_stub() {
        let dir = std::env::temp_dir().join(format!("muretto-d3dc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/core")).unwrap();
        std::fs::write(dir.join("src/core/xul.dll"), "other").unwrap();
        std::fs::write(dir.join("src/core/d3dcompiler_47.dll"), "compiler").unwrap();
        sevenz_rust2::compress_to_path(dir.join("src"), dir.join("archive.7z")).unwrap();

        // A stub that happens to contain the signature, then the archive.
        let mut installer = b"MZ stub 7z\xbc\xaf\x27\x1c not an archive".to_vec();
        installer.extend(std::fs::read(dir.join("archive.7z")).unwrap());
        assert_eq!(extract(&installer).unwrap(), b"compiler");

        assert!(matches!(
            extract(b"MZ nothing here"),
            Err(Error::Archive(_))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_muretto_s_copy_counts_as_installed_and_is_removed() {
        let dir = std::env::temp_dir().join(format!("muretto-d3dc-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("steamapps/common/assettocorsa");
        std::fs::create_dir_all(&root).unwrap();
        let install = Install::at(root.clone(), None);
        assert_eq!(compiler(&install), Compiler::Absent);

        std::fs::write(root.join(FILE), "someone else's").unwrap();
        assert_eq!(compiler(&install), Compiler::Other);
        remove(&install).unwrap();
        assert!(root.join(FILE).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Downloads about 40 MB: `cargo test -p muretto-core -- --ignored`.
    #[test]
    #[ignore]
    fn the_real_installer_yields_the_expected_compiler() {
        let dir = std::env::temp_dir().join(format!("muretto-d3dc-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("steamapps/common/assettocorsa");
        std::fs::create_dir_all(&root).unwrap();
        let install = Install::at(root, None);
        super::install(&install, &Progress::new()).unwrap();
        assert_eq!(compiler(&install), Compiler::Installed);
        remove(&install).unwrap();
        assert_eq!(compiler(&install), Compiler::Absent);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
