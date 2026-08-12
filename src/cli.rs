//! Command-line interface.
//!
//! `clap`'s derive API replaces gostruct's hand-rolled `flag.Usage`: help text,
//! `--version`, typo suggestions and shell-friendly errors all come from the
//! same struct that carries the parsed values.

use std::path::PathBuf;

use clap::Parser;

use crate::config::UserConfig;
use crate::scaffold::ScaffoldOptions;

const AFTER_HELP: &str = concat!(
    "EXAMPLES:\n",
    "  ruststruct myapp\n",
    "  ruststruct --name acme-tool myapp\n",
    "  ruststruct --git myapp\n",
    "  ruststruct --dry-run myapp\n",
    "\n",
    "CONFIG (~/.ruststruct.json, optional):\n",
    "  {\n",
    "    \"dirs\":  [\"scripts\", \"docs\"],\n",
    "    \"files\": { \"rustfmt.toml\": \"edition = \\\"2024\\\"\\n\" }\n",
    "  }\n",
    "  Entries are added to the defaults; a file key that matches a built-in\n",
    "  template replaces it.",
);

/// Scaffold a standard Rust project layout.
#[derive(Debug, Clone, Parser)]
// `about` is intentionally left to the doc comment above: the CLI help reads
// better as an imperative phrase than as the crates.io description.
#[command(name = "ruststruct", version, after_help = AFTER_HELP)]
pub struct Cli {
    /// Directory to create for the new project
    #[arg(value_name = "PROJECT_DIR")]
    pub project_dir: PathBuf,

    /// Cargo package name [default: the project directory name]
    #[arg(short, long, value_name = "NAME")]
    pub name: Option<String>,

    /// Run `git init` once the project is in place
    #[arg(long)]
    pub git: bool,

    /// Print the plan without writing anything
    #[arg(long)]
    pub dry_run: bool,
}

impl Cli {
    /// Turns parsed arguments plus a user config into scaffold options.
    #[must_use]
    pub fn into_options(self, config: UserConfig) -> ScaffoldOptions {
        ScaffoldOptions {
            name: self.name,
            git: self.git,
            dry_run: self.dry_run,
            ..ScaffoldOptions::new(self.project_dir).with_config(config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_the_minimal_invocation() {
        let cli = Cli::parse_from(["ruststruct", "myapp"]);
        assert_eq!(cli.project_dir, PathBuf::from("myapp"));
        assert!(cli.name.is_none());
        assert!(!cli.git);
        assert!(!cli.dry_run);
    }

    #[test]
    fn parses_every_flag() {
        let cli = Cli::parse_from(["ruststruct", "--name", "acme", "--git", "--dry-run", "app"]);
        assert_eq!(cli.name.as_deref(), Some("acme"));
        assert!(cli.git);
        assert!(cli.dry_run);
        assert_eq!(cli.project_dir, PathBuf::from("app"));
    }

    #[test]
    fn missing_project_dir_is_an_error() {
        assert!(Cli::try_parse_from(["ruststruct"]).is_err());
    }

    #[test]
    fn config_is_layered_onto_the_defaults() {
        let config = UserConfig {
            dirs: vec!["scripts".to_string()],
            files: std::collections::BTreeMap::from([("Makefile".to_string(), "all:\n".into())]),
        };
        let opts = Cli::parse_from(["ruststruct", "--git", "app"]).into_options(config);

        assert!(opts.git);
        assert!(opts.dirs.contains(&"src".to_string()));
        assert!(opts.dirs.contains(&"scripts".to_string()));
        assert!(opts.files.contains_key("Makefile"));
    }
}
