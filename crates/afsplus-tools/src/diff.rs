use std::ffi::OsString;
use std::path::PathBuf;

use afsplus_check::diff::{diff_devices, DiffOptions};

use crate::common::{read_header, report_failure, Failure, EXIT_MEDIA, EXIT_OK};

const TOOL: &str = "afsplus-image-diff";
const USAGE: &str = "usage: afsplus-image-diff [--json] [--metadata] <before> <after>";

struct Options {
    before: PathBuf,
    after: PathBuf,
    json: bool,
    metadata: bool,
}

enum ParseResult {
    Options(Box<Options>),
    Help,
}

pub fn run<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    let options = match parse(args) {
        Ok(ParseResult::Options(options)) => options,
        Ok(ParseResult::Help) => {
            println!("{USAGE}");
            println!("Report what the second image differs by from the first in filesystem terms.");
            println!("--metadata compares everything except file content.");
            return EXIT_OK;
        }
        Err(failure) => return report_failure(TOOL, failure),
    };

    match execute(&options) {
        Ok((output, partial)) => {
            println!("{output}");
            // A difference is not a failure; an image that could only be
            // compared in part is one.
            if partial {
                EXIT_MEDIA
            } else {
                EXIT_OK
            }
        }
        Err(failure) => report_failure(TOOL, failure),
    }
}

fn parse<I>(args: I) -> Result<ParseResult, Failure>
where
    I: IntoIterator<Item = OsString>,
{
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut json = false;
    let mut metadata = false;
    let mut positional_only = false;
    for argument in args {
        if !positional_only {
            match argument.to_str() {
                Some("--help" | "-h") => return Ok(ParseResult::Help),
                Some("--json") => {
                    if json {
                        return Err(Failure::usage("--json was specified more than once"));
                    }
                    json = true;
                    continue;
                }
                Some("--metadata") => {
                    if metadata {
                        return Err(Failure::usage("--metadata was specified more than once"));
                    }
                    metadata = true;
                    continue;
                }
                Some("--") => {
                    positional_only = true;
                    continue;
                }
                Some(value) if value.starts_with('-') => {
                    return Err(Failure::usage(format!("unknown option {value:?}; {USAGE}")));
                }
                _ => {}
            }
        }
        paths.push(PathBuf::from(&argument));
    }
    if paths.len() != 2 {
        return Err(Failure::usage(format!(
            "two image paths are required; {USAGE}"
        )));
    }
    let after = paths.pop().expect("two paths");
    let before = paths.pop().expect("two paths");
    Ok(ParseResult::Options(Box::new(Options {
        before,
        after,
        json,
        metadata,
    })))
}

fn execute(options: &Options) -> Result<(String, bool), Failure> {
    let (mut before, _) = read_header(&options.before)?;
    let (mut after, _) = read_header(&options.after)?;
    let diff = diff_devices(
        &mut before,
        &mut after,
        DiffOptions {
            metadata_only: options.metadata,
        },
    )
    .map_err(|error| Failure::media("E_DIFF", format!("cannot compare the images: {error}")))?;
    let output = if options.json {
        diff.render_json()
    } else {
        diff.render_human()
    };
    Ok((output, diff.has_problems()))
}
