//! Binary entry point: parse, load config, scaffold, report.

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use ruststruct::{Cli, UserConfig, scaffold};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `{:#}` renders the whole `source` chain on one line, the way Go's
            // wrapped `%w` errors read.
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let options = Cli::parse().into_options(UserConfig::load()?);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    scaffold(&options, &mut out)?;

    if !options.dry_run {
        writeln!(
            out,
            "Project `{}` created successfully.",
            options.root.display()
        )?;
    }

    Ok(())
}
