//! `ruststruct` scaffolds a standard Rust project layout in one command.
//!
//! The crate is split so that the binary stays a thin shell around a testable
//! library:
//!
//! - [`cli`] — argument parsing ([`clap`]), and the mapping to scaffold options
//! - [`config`] — the optional `~/.ruststruct.json` overrides
//! - [`mod@scaffold`] — planning and applying a layout, with rollback on failure
//! - [`error`] — one typed error per failure mode
//!
//! ```no_run
//! use ruststruct::{ScaffoldOptions, scaffold};
//!
//! let mut opts = ScaffoldOptions::new("myapp");
//! opts.dry_run = true;
//! scaffold(&opts, &mut std::io::stdout())?;
//! # Ok::<(), ruststruct::Error>(())
//! ```

pub mod cli;
pub mod config;
pub mod error;
pub mod scaffold;

pub use cli::Cli;
pub use config::UserConfig;
pub use error::{Error, Result};
pub use scaffold::{DEFAULT_DIRS, ScaffoldOptions, scaffold};
