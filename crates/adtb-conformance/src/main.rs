//! Run the conformance files and the engine scripts, or probe one text.

use std::process::ExitCode;

fn main() -> ExitCode {
    adtb_conformance::runner::cli::main()
}
