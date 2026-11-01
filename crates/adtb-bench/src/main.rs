#[cfg(test)]
extern crate adtb_lang as _;

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    match adtb_bench::native::run(std::env::args().skip(1).collect()) {
        Ok(report) => match writeln!(std::io::stdout(), "{report}") {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        },
        Err(error) => match writeln!(std::io::stderr(), "{error}") {
            Ok(()) => ExitCode::FAILURE,
            Err(_) => ExitCode::from(2),
        },
    }
}
