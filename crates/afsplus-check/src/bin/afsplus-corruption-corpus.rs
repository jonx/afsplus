//! Generate the deterministic checker-corruption corpus.

use std::path::PathBuf;
use std::process::ExitCode;

use afsplus_check::corpus::write_corruption_corpus;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let Some(output) = arguments.next().map(PathBuf::from) else {
        eprintln!("usage: afsplus-corruption-corpus <output-directory>");
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: afsplus-corruption-corpus <output-directory>");
        return ExitCode::from(2);
    }
    match write_corruption_corpus(&output) {
        Ok(count) => {
            println!(
                "generated {count} deterministic corruption cases in {}",
                output.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("afsplus-corruption-corpus: {error}");
            ExitCode::FAILURE
        }
    }
}
