mod cli;
mod config;
mod database;
mod dependency;
mod download;
mod error;
mod install;
mod manifest;
mod package;
mod remove;
mod repository;
mod signature;
mod transaction;
mod upgrade;
mod verify;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    match cli::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}