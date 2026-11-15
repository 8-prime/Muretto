//! Everything Muretto knows about Assetto Corsa that is not user interface:
//! where the game lives, what content is installed, and how to start a session.

pub mod content;
pub mod country;
pub mod group;
pub mod ini;
pub mod install;
pub mod launch;
pub mod lenient;
pub mod online;
pub mod packages;
pub mod session;
pub mod settings;
pub mod steam;

pub use content::{Car, Library, Origin, Skin, Track, TrackLayout, Weather};
pub use install::Install;
pub use session::{SessionConfig, SessionMode};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Parse {
        path: std::path::PathBuf,
        message: String,
    },
    #[error("Assetto Corsa was not found in any Steam library")]
    InstallNotFound,
    #[error("{0}")]
    Launch(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Archive(String),
    #[error("Cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io_err(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}
