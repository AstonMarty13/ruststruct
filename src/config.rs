//! Optional user-level configuration, read from `~/.ruststruct.json`.
//!
//! The file is entirely optional: a missing file yields an empty config, and
//! only a *malformed* file is an error. Unknown keys are rejected on purpose —
//! a typo such as `"dir"` instead of `"dirs"` should be reported, not silently
//! ignored.
//!
//! ```json
//! {
//!   "dirs":  ["scripts", "docs"],
//!   "files": { "rustfmt.toml": "edition = \"2024\"\n" }
//! }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Extra directories and files to add on top of the built-in defaults.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    /// Additional directories to create inside the project root.
    #[serde(default)]
    pub dirs: Vec<String>,

    /// Additional files to write, keyed by path relative to the project root.
    ///
    /// An entry whose key matches a built-in file overrides it.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

impl UserConfig {
    /// Name of the config file, looked up in the user's home directory.
    pub const FILE_NAME: &'static str = ".ruststruct.json";

    /// Loads the config from `~/.ruststruct.json`.
    ///
    /// Returns [`UserConfig::default`] when the home directory cannot be
    /// determined or the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ReadConfig`] if the file exists but cannot be read, and
    /// [`Error::ParseConfig`] if it is not valid JSON for this shape.
    pub fn load() -> Result<Self> {
        let Some(home) = home_dir() else {
            return Ok(Self::default());
        };
        Self::load_from(&home.join(Self::FILE_NAME))
    }

    /// Loads the config from an explicit path.
    ///
    /// A missing file is not an error — it yields [`UserConfig::default`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::ReadConfig`] if the file exists but cannot be read, and
    /// [`Error::ParseConfig`] if it is not valid JSON for this shape.
    pub fn load_from(path: &Path) -> Result<Self> {
        let data = match std::fs::read_to_string(path) {
            Ok(data) => data,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(source) => {
                return Err(Error::ReadConfig {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        serde_json::from_str(&data).map_err(|source| Error::ParseConfig {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Returns `true` when the config adds nothing to the defaults.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty() && self.files.is_empty()
    }
}

/// Resolves the user's home directory.
///
/// `std::env::home_dir` honours `$HOME` on Unix, which keeps the lookup
/// overridable from tests.
fn home_dir() -> Option<PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_default() {
        let cfg = UserConfig::load_from(Path::new("/definitely/not/here.json")).unwrap();
        assert_eq!(cfg, UserConfig::default());
        assert!(cfg.is_empty());
    }

    #[test]
    fn parses_dirs_and_files() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join(UserConfig::FILE_NAME);
        std::fs::write(
            &path,
            r#"{ "dirs": ["scripts", "docs"], "files": { "Makefile": "build:\n" } }"#,
        )
        .unwrap();

        let cfg = UserConfig::load_from(&path).unwrap();
        assert_eq!(cfg.dirs, ["scripts", "docs"]);
        assert_eq!(cfg.files["Makefile"], "build:\n");
    }

    #[test]
    fn partial_config_is_allowed() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join(UserConfig::FILE_NAME);
        std::fs::write(&path, r#"{ "dirs": ["only-dirs"] }"#).unwrap();

        let cfg = UserConfig::load_from(&path).unwrap();
        assert_eq!(cfg.dirs, ["only-dirs"]);
        assert!(cfg.files.is_empty());
    }

    #[test]
    fn unknown_key_is_rejected() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join(UserConfig::FILE_NAME);
        // `dir` instead of `dirs`: Go's encoding/json would silently ignore this.
        std::fs::write(&path, r#"{ "dir": ["scripts"] }"#).unwrap();

        let err = UserConfig::load_from(&path).unwrap_err();
        assert!(matches!(err, Error::ParseConfig { .. }), "got {err:?}");
    }

    #[test]
    fn malformed_json_is_rejected() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join(UserConfig::FILE_NAME);
        std::fs::write(&path, "{ not json").unwrap();

        assert!(matches!(
            UserConfig::load_from(&path).unwrap_err(),
            Error::ParseConfig { .. }
        ));
    }

    #[test]
    fn round_trips_through_json() {
        let cfg = UserConfig {
            dirs: vec!["scripts".into(), "deployments".into()],
            files: BTreeMap::from([("Makefile".to_string(), "build:\n".to_string())]),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<UserConfig>(&json).unwrap(), cfg);
    }

    // `UserConfig::load()` reads `$HOME`, and `env::set_var` is `unsafe` in
    // edition 2024 — which this crate forbids. It is covered instead in
    // `tests/cli.rs`, where `HOME` is set on the child process.
}
