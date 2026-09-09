//! Server executable composition root and process lifecycle.

mod args;
mod logging;
mod run;
mod settings_file;

use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    match args::Args::parse(std::env::args_os().skip(1)).and_then(|args| run::run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("{error}");
            let _ = writeln!(io::stderr(), "muxy-server: {error}");
            ExitCode::FAILURE
        }
    }
}
