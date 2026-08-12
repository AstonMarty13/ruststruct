//! Typed errors for every fallible step of a scaffold run.
//!
//! Each variant keeps the data that caused the failure (a path, a command line)
//! instead of flattening it into a string, so callers can match on the cause
//! rather than parse a message.

use std::path::PathBuf;

/// Everything that can go wrong while scaffolding a project.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The target directory already exists; refusing to touch it.
    #[error("directory `{}` already exists", .0.display())]
    RootExists(PathBuf),

    /// No package name was given and none could be derived from the path.
    #[error("cannot derive a package name from `{}`; pass --name", .0.display())]
    UnnamedRoot(PathBuf),

    /// A path coming from the user config would escape the project root.
    ///
    /// Guards against a `~/.ruststruct.json` containing entries such as
    /// `"../../.zshrc"` or `"/etc/passwd"`.
    #[error("path `{}` is not a plain relative path inside the project", .0.display())]
    UnsafePath(PathBuf),

    /// A directory could not be created.
    #[error("creating directory `{}`", .path.display())]
    CreateDir {
        /// Directory that could not be created.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A file could not be written.
    #[error("writing file `{}`", .path.display())]
    WriteFile {
        /// File that could not be written.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The user config file exists but could not be read.
    #[error("reading config `{}`", .path.display())]
    ReadConfig {
        /// Config file that could not be read.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The user config file is not valid JSON, or has unexpected keys.
    #[error("parsing config `{}`", .path.display())]
    ParseConfig {
        /// Config file that could not be parsed.
        path: PathBuf,
        /// Underlying deserialization failure.
        #[source]
        source: serde_json::Error,
    },

    /// An external program could not be started at all (usually: not on `PATH`).
    #[error("running `{command}` (is it installed and on your PATH?)")]
    CommandSpawn {
        /// The command line that could not be spawned.
        command: String,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// An external program ran but exited with a non-zero status.
    #[error("`{command}` failed ({status})\n{output}")]
    CommandFailed {
        /// The command line that failed.
        command: String,
        /// Its exit status, rendered for humans.
        status: String,
        /// Its combined stdout + stderr, trimmed.
        output: String,
    },

    /// The dry-run plan could not be written to the output stream.
    #[error("writing the dry-run plan")]
    Output(#[from] std::io::Error),
}

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
