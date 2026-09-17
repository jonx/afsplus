use std::ffi::OsString;
use std::path::PathBuf;

use afsplus_core::volume::emergency_headroom_for_volume;

use crate::common::{
    feature_names, features_json, header_json, name_algorithm_name, read_header, report_failure,
    uuid_hex, Failure, EXIT_OK,
};

const TOOL: &str = "afsplus-info";
const USAGE: &str = "usage: afsplus-info [--json] <image>";

struct Options {
    image: PathBuf,
    json: bool,
}

enum ParseResult {
    Options(Options),
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
            println!("Read immutable identification and select a checkpoint without writing.");
            return EXIT_OK;
        }
        Err(failure) => return report_failure(TOOL, failure),
    };

    match execute(&options) {
        Ok(output) => {
            println!("{output}");
            EXIT_OK
        }
        Err(failure) => report_failure(TOOL, failure),
    }
}

fn parse<I>(args: I) -> Result<ParseResult, Failure>
where
    I: IntoIterator<Item = OsString>,
{
    let mut image = None;
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
        if image.replace(PathBuf::from(&argument)).is_some() {
            return Err(Failure::usage(format!(
                "exactly one image path is required; {USAGE}"
            )));
        }
    }
    let image = image.ok_or_else(|| Failure::usage(format!("missing image path; {USAGE}")))?;
    Ok(ParseResult::Options(Options { image, json }))
}

fn execute(options: &Options) -> Result<String, Failure> {
    let (_device, view) = read_header(&options.image)?;
    if options.json {
        return Ok(format!(
            "{{\"schema_version\":1,\"tool\":\"afsplus-info\",{}}}",
            header_json(&view)
        ));
    }

    let ident = &view.ident;
    let checkpoint = &view.selection.chosen;
    let emergency_headroom = emergency_headroom_for_volume(ident.total_blocks);
    let available_blocks = checkpoint
        .free_blocks_total
        .saturating_sub(emergency_headroom);
    let feature_list = feature_names(ident);
    let features = if feature_list.is_empty() {
        "none".to_owned()
    } else {
        feature_list.join(", ")
    };
    Ok(format!(
        "AFS+ volume {}\nlabel: {:?}\ngeometry: {} blocks x {} bytes; region {} blocks\nnames: {} (Unicode {}.{}.{})\nfeatures: {}\nfeature masks: {}\ncheckpoint: slot {}, generation {}, transaction {}, raw free {}, emergency headroom {}, normally available {} blocks\nslot A: {}\nslot B: {}",
        uuid_hex(&ident.uuid),
        checkpoint.label,
        ident.total_blocks,
        ident.block_size(),
        ident.region_size,
        name_algorithm_name(ident.name_key_algorithm),
        ident.unicode_version[0],
        ident.unicode_version[1],
        ident.unicode_version[2],
        features,
        features_json(ident),
        if view.selection.chosen_slot == 0 { "A" } else { "B" },
        checkpoint.generation,
        checkpoint.committed_tx_id,
        checkpoint.free_blocks_total,
        emergency_headroom,
        available_blocks,
        view.selection.slot_status[0],
        view.selection.slot_status[1],
    ))
}
