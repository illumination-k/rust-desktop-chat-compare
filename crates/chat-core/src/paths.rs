use std::path::PathBuf;

use crate::{Error, Result};

/// Overrides the root directory for data and config (benchmarks, tests, portable use).
pub const HOME_ENV: &str = "CHAT_COMPARE_HOME";

/// OS-standard locations shared by every app, so the same history can be
/// opened in each framework for a fair comparison.
#[derive(Clone, Debug)]
pub struct AppPaths {
    pub conversations_dir: PathBuf,
    pub settings_file: PathBuf,
}

impl AppPaths {
    pub fn from_env() -> Result<Self> {
        if let Some(home) = std::env::var_os(HOME_ENV) {
            return Ok(Self::under(PathBuf::from(home)));
        }
        let dirs =
            directories::ProjectDirs::from("dev", "illumination-k", "rust-desktop-chat-compare")
                .ok_or(Error::NoHomeDir)?;
        Ok(Self {
            conversations_dir: dirs.data_dir().join("conversations"),
            settings_file: dirs.config_dir().join("settings.json"),
        })
    }

    pub fn under(root: PathBuf) -> Self {
        Self {
            conversations_dir: root.join("conversations"),
            settings_file: root.join("settings.json"),
        }
    }
}
