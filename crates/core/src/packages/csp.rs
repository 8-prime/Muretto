//! Custom Shaders Patch: finding out what is installed, what is on offer,
//! and installing or removing it.
//!
//! The patch is a `dwrite.dll` the game loads in place of the system's, plus
//! an `extension` folder of data. Free builds are downloaded from the
//! author's site; preview builds are given to supporters as the same kind of
//! archive and are installed from the file.

use std::path::Path;

use super::{Progress, Receipt, Replaced, Staged, download, downloads_dir, fetch_text, uninstall};
use crate::{Error, Install, Result, ini::Ini, io_err, lenient};

pub const ID: &str = "csp";
const PAGE: &str = "https://acstuff.club/patch/";
const LOADER: &str = "dwrite.dll";
const MANIFEST: &str = "extension/config/data_manifest.ini";

/// The patch as found in the game folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub version: Option<String>,
    pub build: Option<String>,
    /// Installed by Muretto, which therefore knows every file to remove.
    pub managed: bool,
}

/// How far the author trusts a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    Recommended,
    Untested,
    Buggy,
    Other(String),
}

impl Standing {
    pub fn label(&self) -> &str {
        match self {
            Standing::Recommended => "Recommended",
            Standing::Untested => "Untested",
            Standing::Buggy => "Buggy",
            Standing::Other(label) => label,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub size_bytes: Option<u64>,
    pub standing: Standing,
}

/// The builds on offer, newest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalogue {
    pub releases: Vec<Release>,
}

impl Catalogue {
    /// The newest build the author recommends.
    pub fn recommended(&self) -> Option<&Release> {
        self.releases
            .iter()
            .find(|release| release.standing == Standing::Recommended)
    }

    pub fn latest(&self) -> Option<&Release> {
        self.releases.first()
    }

    /// The builds known when this version of Muretto was made, for when the
    /// download page cannot be read.
    pub fn built_in() -> Self {
        let release = |version: &str, megabytes: f64, standing| Release {
            version: version.to_owned(),
            size_bytes: Some((megabytes * 1e6) as u64),
            standing,
        };
        Catalogue {
            releases: vec![
                release("0.3.1", 140.76, Standing::Untested),
                release("0.2.11", 119.12, Standing::Recommended),
            ],
        }
    }

    /// Reads the version list of the download page. Each entry looks like
    /// `<a href="?info=0.2.11">v0.2.11</a> (119.12 MB, <span class="tag
    /// tag-recommended">recommended</span>)`.
    pub fn parse(page: &str) -> Self {
        let mut releases: Vec<Release> = page
            .split("?info=")
            .skip(1)
            .filter_map(|entry| {
                let entry = entry.split("</li>").next()?;
                let version = entry.split('"').next()?;
                if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    return None;
                }
                let size_bytes = entry
                    .split_once(" MB")
                    .and_then(|(before, _)| before.rsplit('(').next())
                    .and_then(|megabytes| megabytes.trim().parse::<f64>().ok())
                    .map(|megabytes| (megabytes * 1e6) as u64);
                let standing = match entry
                    .split("tag tag-")
                    .nth(1)
                    .and_then(|tag| tag.split('"').next())
                {
                    Some("recommended") => Standing::Recommended,
                    Some("untested") | None => Standing::Untested,
                    Some("buggy") => Standing::Buggy,
                    Some(other) => Standing::Other(capitalize(other)),
                };
                Some(Release {
                    version: version.to_owned(),
                    size_bytes,
                    standing,
                })
            })
            .collect();
        releases.sort_by_key(|release| std::cmp::Reverse(version_key(&release.version)));
        releases.dedup_by(|a, b| a.version == b.version);
        Catalogue { releases }
    }
}

/// Where builds come from. The real one is [`Official`]; tests supply their
/// own.
pub trait Source: Send + Sync {
    fn catalogue(&self) -> Result<Catalogue>;
    /// Downloads the archive of `release` to `to`.
    fn fetch(&self, release: &Release, to: &Path, progress: &Progress) -> Result<()>;
}

/// The author's download site.
pub struct Official;

impl Source for Official {
    fn catalogue(&self) -> Result<Catalogue> {
        let catalogue = Catalogue::parse(&fetch_text(PAGE)?);
        if catalogue.releases.is_empty() {
            return Err(Error::Network(
                "The download page did not list any versions.".to_owned(),
            ));
        }
        Ok(catalogue)
    }

    fn fetch(&self, release: &Release, to: &Path, progress: &Progress) -> Result<()> {
        let url = format!("{PAGE}?get={}", release.version);
        download(&url, to, release.size_bytes, progress)
    }
}

/// The patch in this game folder, if its loader and data are both present.
pub fn installed(install: &Install) -> Option<Installed> {
    let root = &install.root;
    if !root.join(LOADER).is_file() || !root.join("extension").is_dir() {
        return None;
    }
    let (version, build) = std::fs::read(root.join(MANIFEST))
        .ok()
        .map(|bytes| version_of(&bytes))
        .unwrap_or_default();
    Some(Installed {
        version,
        build,
        managed: Receipt::load(root, ID).is_some(),
    })
}

/// Downloads `release` and installs it over whatever is there.
pub fn install(
    install: &Install,
    source: &dyn Source,
    release: &Release,
    progress: &Progress,
) -> Result<Installed> {
    let archive = downloads_dir(&install.root).join(format!("csp-{}.zip", release.version));
    let result = source
        .fetch(release, &archive, progress)
        .and_then(|()| install_from_file(install, &archive, progress));
    let _ = std::fs::remove_file(&archive);
    let _ = std::fs::remove_dir(downloads_dir(&install.root));
    result
}

/// Installs a patch archive the player already has, such as a preview build.
pub fn install_from_file(
    install: &Install,
    archive: &Path,
    progress: &Progress,
) -> Result<Installed> {
    let belongs = |path: &str| path == LOADER || path.starts_with("extension/");
    let staged = Staged::unpack(&install.root, ID, archive, belongs, progress)?;
    let version = staged
        .read(MANIFEST)
        .and_then(|bytes| version_of(&bytes).0)
        .filter(|_| staged.contains(LOADER))
        .ok_or_else(|| {
            Error::Archive(
                "This archive is not a Custom Shaders Patch build: it has no dwrite.dll or no version file."
                    .to_owned(),
            )
        })?;
    staged.install(ID, &version, Replaced::Discarded, progress)?;
    installed(install)
        .ok_or_else(|| Error::Archive("The patch did not install correctly.".to_owned()))
}

/// Removes the patch. A copy Muretto installed is removed completely. One
/// installed by other means is only switched off, by deleting its loader:
/// its `extension` folder may hold other mods that Muretto cannot tell apart.
pub fn remove(install: &Install) -> Result<()> {
    if uninstall(&install.root, ID)? {
        return Ok(());
    }
    let loader = install.root.join(LOADER);
    match std::fs::remove_file(&loader) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io_err(&loader)(e)),
        _ => Ok(()),
    }
}

/// (version, build) from a `data_manifest.ini`.
fn version_of(manifest: &[u8]) -> (Option<String>, Option<String>) {
    let ini = Ini::parse(&lenient::decode(manifest));
    let value = |key| {
        ini.get("VERSION", key)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    (value("SHADERS_PATCH"), value("SHADERS_PATCH_BUILD"))
}

/// Orders "0.2.9" before "0.2.11".
pub fn version_key(version: &str) -> Vec<u32> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{temp_dir, zip_of};
    use super::*;

    const PAGE_EXCERPT: &str = r#"<li><a download="v0.2.11.zip" href="?get=0.2.11" style="color:#0f0">v0.2.11 (recommended version)</a></li>
      <ul class="previous-versions" style="display:none"><li><a target="_blank" href="?info=0.1.79">v0.1.79</a> (80.74 MB, <span class="tag tag-recommended">recommended</span>)</li><li><a target="_blank" href="?info=0.2.9">v0.2.9</a> (118.28 MB, <span class="tag tag-buggy">buggy</span>)</li><li><a target="_blank" href="?info=0.2.11">v0.2.11</a> (119.12 MB, <span class="tag tag-recommended">recommended</span>)</li><li><a target="_blank" href="?info=0.3.1">v0.3.1</a> (140.76 MB, <span class="tag tag-untested">untested</span>)</li></ul>"#;

    const MANIFEST_TEXT: &str =
        "; For launchers\n[VERSION]\n; hidden\nSHADERS_PATCH=0.2.11\nSHADERS_PATCH_BUILD=3465\n";

    struct Shelf {
        archive: std::path::PathBuf,
    }

    impl Source for Shelf {
        fn catalogue(&self) -> Result<Catalogue> {
            Ok(Catalogue::parse(PAGE_EXCERPT))
        }

        fn fetch(&self, _: &Release, to: &Path, _: &Progress) -> Result<()> {
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(&self.archive, to).unwrap();
            Ok(())
        }
    }

    #[test]
    fn the_download_page_lists_builds_newest_first() {
        let catalogue = Catalogue::parse(PAGE_EXCERPT);
        let versions: Vec<_> = catalogue
            .releases
            .iter()
            .map(|r| r.version.as_str())
            .collect();
        assert_eq!(versions, ["0.3.1", "0.2.11", "0.2.9", "0.1.79"]);
        assert_eq!(catalogue.latest().unwrap().version, "0.3.1");
        let recommended = catalogue.recommended().unwrap();
        assert_eq!(recommended.version, "0.2.11");
        assert_eq!(recommended.size_bytes, Some(119_120_000));
        assert_eq!(catalogue.releases[2].standing, Standing::Buggy);
        assert!(
            Catalogue::parse("<html>maintenance</html>")
                .releases
                .is_empty()
        );
    }

    #[test]
    fn the_built_in_list_agrees_with_the_page_it_was_copied_from() {
        let built_in = Catalogue::built_in();
        assert_eq!(built_in.recommended().unwrap().version, "0.2.11");
        assert_eq!(built_in.latest().unwrap().version, "0.3.1");
    }

    #[test]
    fn a_build_is_installed_recognised_and_removed() {
        let dir = temp_dir("csp");
        let install = Install::at(dir.join("steamapps/common/assettocorsa"), None);
        std::fs::create_dir_all(&install.root).unwrap();
        assert_eq!(installed(&install), None);

        let archive = dir.join("build.zip");
        zip_of(
            &archive,
            &[
                ("dwrite.dll", "loader"),
                (MANIFEST, MANIFEST_TEXT),
                ("extension/shaders.zip", "data"),
            ],
        );
        let source = Shelf { archive };
        let release = source.catalogue().unwrap().recommended().unwrap().clone();
        let patch = install_patch(&install, &source, &release);
        assert_eq!(
            patch,
            Installed {
                version: Some("0.2.11".to_owned()),
                build: Some("3465".to_owned()),
                managed: true,
            }
        );
        assert_eq!(installed(&install), Some(patch));
        assert!(
            !install.root.join(".muretto/downloads").exists(),
            "the download is cleaned up"
        );

        remove(&install).unwrap();
        assert_eq!(installed(&install), None);
        assert!(!install.root.join("extension").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn install_patch(game: &Install, source: &dyn Source, release: &Release) -> Installed {
        install(game, source, release, &Progress::new()).unwrap()
    }

    #[test]
    fn a_copy_installed_by_other_means_is_only_switched_off() {
        let dir = temp_dir("csp-foreign");
        let install = Install::at(dir.join("steamapps/common/assettocorsa"), None);
        std::fs::create_dir_all(install.root.join("extension/weather/pure")).unwrap();
        std::fs::write(install.root.join("dwrite.dll"), "loader").unwrap();
        assert_eq!(
            installed(&install),
            Some(Installed {
                version: None,
                build: None,
                managed: false,
            })
        );

        remove(&install).unwrap();
        assert_eq!(installed(&install), None);
        assert!(install.root.join("extension/weather/pure").is_dir());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_archive_that_is_not_the_patch_changes_nothing() {
        let dir = temp_dir("csp-wrong");
        let install = Install::at(dir.join("steamapps/common/assettocorsa"), None);
        std::fs::create_dir_all(&install.root).unwrap();
        // Looks like a mod for the patch, but has no loader.
        zip_of(
            &dir.join("mod.zip"),
            &[("extension/weather/pure/a.lua", "x")],
        );
        let result = install_from_file(&install, &dir.join("mod.zip"), &Progress::new());
        assert!(
            matches!(result, Err(Error::Archive(m)) if m.contains("not a Custom Shaders Patch"))
        );
        assert!(!install.root.join("extension").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
