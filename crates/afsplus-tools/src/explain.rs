use std::ffi::OsString;
use std::path::PathBuf;

use afsplus_check::explain::Explainer;
use afsplus_check::explain_render::{
    render_block_human, render_block_json, render_object_human, render_object_json,
    render_path_human, render_path_json,
};

use crate::common::{read_header, report_failure, Failure, EXIT_MEDIA, EXIT_OK};

const TOOL: &str = "afsplus-explain";
const USAGE: &str =
    "usage: afsplus-explain [--json] <image> (block <number> | object <id> | path <path>)";

enum Question {
    Block(u64),
    Object(u64),
    Path(String),
}

struct Options {
    image: PathBuf,
    question: Question,
    json: bool,
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
            println!("Say what the committed state of an image believes about one block,");
            println!("one object or one path. The image is opened read-only.");
            println!("A path is absolute inside the volume, with / between components.");
            return EXIT_OK;
        }
        Err(failure) => return report_failure(TOOL, failure),
    };
    match execute(&options) {
        Ok((output, partial)) => {
            print!("{output}");
            if !output.ends_with('\n') {
                println!();
            }
            // An answer from a walk that could not decode every branch is
            // an answer about a damaged image.
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
    let mut positional: Vec<OsString> = Vec::new();
    let mut json = false;
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
        positional.push(argument);
    }
    let [image, kind, value] = <[OsString; 3]>::try_from(positional)
        .map_err(|_| Failure::usage(format!("an image and one question are required; {USAGE}")))?;
    let text = value
        .to_str()
        .ok_or_else(|| Failure::usage("the question value is not valid UTF-8"))?;
    let number = |what: &str| {
        text.parse::<u64>()
            .map_err(|_| Failure::usage(format!("{what} {text:?} is not a decimal number")))
    };
    let question = match kind.to_str() {
        Some("block") => Question::Block(number("block number")?),
        Some("object") => Question::Object(number("object ID")?),
        Some("path") => Question::Path(text.to_owned()),
        _ => {
            return Err(Failure::usage(format!(
                "the question is block, object or path; {USAGE}"
            )))
        }
    };
    Ok(ParseResult::Options(Box::new(Options {
        image: PathBuf::from(image),
        question,
        json,
    })))
}

fn execute(options: &Options) -> Result<(String, bool), Failure> {
    let (mut image, _) = read_header(&options.image)?;
    let explainer = Explainer::load(&mut image)
        .map_err(|error| Failure::media("E_EXPLAIN", format!("cannot read the image: {error}")))?;
    let not_found = |error: String| Failure::media("E_NOT_FOUND", error);
    let output = match &options.question {
        Question::Block(lba) => {
            let block = explainer
                .explain_block(&mut image, *lba)
                .map_err(not_found)?;
            if options.json {
                render_block_json(&explainer, &block)
            } else {
                render_block_human(&explainer, &block)
            }
        }
        Question::Object(id) => {
            let object = explainer.explain_object(*id).map_err(not_found)?;
            if options.json {
                render_object_json(&explainer, object)
            } else {
                render_object_human(&explainer, object)
            }
        }
        Question::Path(path) => {
            let resolved = explainer.explain_path(path).map_err(not_found)?;
            if options.json {
                render_path_json(&explainer, &resolved)
            } else {
                render_path_human(&explainer, &resolved)
            }
        }
    };
    Ok((output, !explainer.problems.is_empty()))
}
