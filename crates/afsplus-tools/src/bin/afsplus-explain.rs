use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(afsplus_tools::run_explain(std::env::args_os().skip(1)))
}
