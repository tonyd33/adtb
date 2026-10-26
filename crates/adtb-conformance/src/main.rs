use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut failed = false;
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        let found = if path.is_dir() {
            adtb_conformance::run_dir(path)
        } else {
            adtb_conformance::run_file(path)
        };
        for d in &found {
            println!("{d}");
        }
        failed |= !found.is_empty();
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
