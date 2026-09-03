use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(afsplus_tools::run_dump(std::env::args_os().skip(1)))
}
