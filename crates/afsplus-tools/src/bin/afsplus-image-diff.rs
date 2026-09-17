use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(afsplus_tools::run_image_diff(std::env::args_os().skip(1)))
}
