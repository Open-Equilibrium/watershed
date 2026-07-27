//! Flow Agent command-line entry point.

mod dispatch;
mod output;
mod parsing;
mod streaming;
mod tail;

#[cfg(test)]
#[path = "../../tests/support.rs"]
mod test_support;

use crate::{
    dispatch::dispatch,
    output::{print_error, write_stdout},
    parsing::{informational_output, parse_args},
};
use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let args = match parse_args(env::args_os().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            print_error(&err);
            return ExitCode::from(64);
        }
    };

    if let Some(contents) = informational_output(&args) {
        return match write_stdout(&contents) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                print_error(&err);
                ExitCode::from(err.exit_code() as u8)
            }
        };
    }

    match dispatch(&args) {
        Ok(code) => code,
        Err(err) => {
            print_error(&err);
            ExitCode::from(err.exit_code() as u8)
        }
    }
}
