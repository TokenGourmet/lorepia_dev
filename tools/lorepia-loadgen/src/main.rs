#![forbid(unsafe_code)]

mod args;
mod assets;
mod bench;
mod db;
mod stream;
mod util;
mod verify;

use std::{env, process::ExitCode};

use args::{Command, ParsedArgs};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lorepia-loadgen: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> util::Result<()> {
    let parsed = ParsedArgs::parse(env::args_os().skip(1))?;
    match parsed.command {
        Command::Db(options) => db::generate(options),
        Command::Assets(options) => assets::generate(options),
        Command::Stream(options) => stream::generate(options),
        Command::Verify(options) => verify::run(options),
        Command::Bench(options) => bench::run(options),
        Command::Help => {
            print!("{}", args::HELP);
            Ok(())
        }
    }
}
