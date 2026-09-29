use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::store::{read_json, write_json};

const KEYRING_SERVICE: &str = "rust-desktop-chat-compare";
const KEYRING_USER: &str = "anthropic-api-key";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Anthropic,
    /// Fixed response streamed at a constant rate; used for benchmarks.
    Mock,
}

impl ProviderKind {
    pub const ALL: [Self; 2] = [Self::Anthropic, Self::Mock];

    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::Mock => "Mock",
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// User settings persisted as JSON. The API key is *not* part of this; see [`ApiKeyStore`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub provider: ProviderKind,
    pub model: String,
    pub system_prompt: String,
    pub max_tokens: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: ProviderKind::default(),
            model: "claude-sonnet-5-5".to_owned(),
            system_prompt: "You are a helpful assistant.".to_owned(),
            max_tokens: 4096,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns defaults when the file does not exist yet.
    pub fn load(&self) -> Result<Settings> {
        if !self.path.exists() {
            return Ok(Settings::default());
        }
        read_json(&self.path)
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        write_json(&self.path, settings)
    }
}

/// API key storage in the OS keychain (never written to disk in plain text).
#[derive(Clone, Copy, Debug, Default)]
pub struct ApiKeyStore;

impl ApiKeyStore {
    pub fn get(self) -> Result<Option<String>> {
        match entry()?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn is_set(self) -> bool {
        matches!(self.get(), Ok(Some(_)))
    }

    pub fn set(self, key: &str) -> Result<()> {
        Ok(entry()?.set_password(key.trim())?)
    }

    pub fn delete(self) -> Result<()> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("s/settings.json"));
        assert_eq!(store.load().unwrap(), Settings::default());

        let settings = Settings {
            provider: ProviderKind::Mock,
            model: "m".into(),
            system_prompt: "p".into(),
            max_tokens: 1,
        };
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        let parsed: Settings = serde_json::from_str(r#"{"provider":"mock","extra":1}"#).unwrap();
        assert_eq!(parsed.provider, ProviderKind::Mock);
        assert_eq!(parsed.model, Settings::default().model);
    }
}
