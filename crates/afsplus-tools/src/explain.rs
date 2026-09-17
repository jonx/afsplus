use std::ffi::OsString;
use std::path::PathBuf;

use afsplus_check::explain::Explainer;
use afsplus_check::explain_render::{
    render_block_human, render_block_json, render_checkpoint_human, render_checkpoint_json,
    render_extent_human, render_extent_json, render_features_human, render_features_json,
    render_object_human, render_object_json, render_path_human, render_path_json,
    render_reclaim_human, render_reclaim_json, render_space_human, render_space_json,
};

use crate::common::{read_header, report_failure, Failure, EXIT_MEDIA, EXIT_OK};

const TOOL: &str = "afsplus-explain";
const USAGE: &str = "usage: afsplus-explain [--json] <image> (block <number> | object <id> | path <path> | extent <id> <offset> | checkpoint | reclaim [<block>] | space <region> | feature [<id>])";

enum Question {
    Block(u64),
    Object(u64),
    Path(String),
    Extent(u64, u64),
    Checkpoint,
    Reclaim(Option<u64>),
    Space(u32),
    Feature(Option<String>),
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
            println!("Say what the committed state of an image believes. The image is");
            println!("opened read-only.");
            println!("A path is absolute inside the volume, with / between components.");
            println!("extent asks where one byte offset of a file lives; reclaim with a block");
            println!("asks for the pending run that holds it; feature takes a registry identity.");
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
    let mut positional = positional.into_iter();
    let (Some(image), Some(kind)) = (positional.next(), positional.next()) else {
        return Err(Failure::usage(format!(
            "an image and one question are required; {USAGE}"
        )));
    };
    let mut values = Vec::new();
    for value in positional {
        values.push(
            value
                .into_string()
                .map_err(|_| Failure::usage("a question value is not valid UTF-8"))?,
        );
    }
    let number = |what: &str, text: &String| {
        text.parse::<u64>()
            .map_err(|_| Failure::usage(format!("{what} {text:?} is not a decimal number")))
    };
    let question = match (kind.to_str(), values.as_slice()) {
        (Some("block"), [block]) => Question::Block(number("block number", block)?),
        (Some("object"), [id]) => Question::Object(number("object ID", id)?),
        (Some("path"), [path]) => Question::Path(path.clone()),
        (Some("extent"), [id, offset]) => {
            Question::Extent(number("object ID", id)?, number("offset", offset)?)
        }
        (Some("checkpoint"), []) => Question::Checkpoint,
        (Some("reclaim"), []) => Question::Reclaim(None),
        (Some("reclaim"), [block]) => Question::Reclaim(Some(number("block number", block)?)),
        (Some("space"), [region]) => Question::Space(
            u32::try_from(number("region", region)?)
                .map_err(|_| Failure::usage("the region number is out of range"))?,
        ),
        (Some("feature"), []) => Question::Feature(None),
        (Some("feature"), [id]) => Question::Feature(Some(id.clone())),
        _ => {
            return Err(Failure::usage(format!(
                "unknown question or wrong number of values; {USAGE}"
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
        Question::Extent(id, offset) => {
            let extent = explainer.explain_extent(*id, *offset).map_err(not_found)?;
            if options.json {
                render_extent_json(&explainer, &extent)
            } else {
                render_extent_human(&explainer, &extent)
            }
        }
        Question::Checkpoint => {
            let checkpoint = explainer.explain_checkpoint();
            if options.json {
                render_checkpoint_json(&explainer, &checkpoint)
            } else {
                render_checkpoint_human(&explainer, &checkpoint)
            }
        }
        Question::Reclaim(block) => {
            let reclaim = explainer.explain_reclaim(*block).map_err(not_found)?;
            if options.json {
                render_reclaim_json(&explainer, &reclaim)
            } else {
                render_reclaim_human(&explainer, &reclaim)
            }
        }
        Question::Space(region) => {
            let space = explainer.explain_space(*region).map_err(not_found)?;
            if options.json {
                render_space_json(&explainer, &space)
            } else {
                render_space_human(&explainer, &space)
            }
        }
        Question::Feature(id) => {
            let features = match id {
                Some(id) => vec![explainer.explain_feature(id).map_err(not_found)?],
                None => explainer.explain_features(),
            };
            if options.json {
                render_features_json(&explainer, &features)
            } else {
                render_features_human(&explainer, &features)
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
