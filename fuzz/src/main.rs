use std::path::PathBuf;
use std::process::ExitCode;

use afsplus_format_fuzz::{
    exercise, read_artifact, run_case, write_artifact, CodecTarget, FuzzArtifact,
    SEED_SCHEMA_VERSION,
};

const DEFAULT_RUNS: u64 = 4096;

struct Options {
    runs: u64,
    target: Option<CodecTarget>,
    case: Option<u64>,
    progress: Option<PathBuf>,
    artifact: Option<PathBuf>,
    replay: Option<PathBuf>,
}

fn usage() -> &'static str {
    "usage: afsplus-format-fuzz [--runs N] [--target NAME] [--case N] \
     [--progress FILE] [--artifact FILE]\n\
     afsplus-format-fuzz --replay FILE\n\
     targets: identification checkpoint tree-node object-record intent-log"
}

fn value(arguments: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn parse() -> Result<Options, String> {
    let mut options = Options {
        runs: DEFAULT_RUNS,
        target: None,
        case: None,
        progress: None,
        artifact: None,
        replay: None,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--runs" => {
                options.runs = value(&mut arguments, "--runs")?
                    .parse()
                    .map_err(|_| "--runs must be a positive integer".to_owned())?;
                if options.runs == 0 {
                    return Err("--runs must not be zero".into());
                }
            }
            "--target" => {
                let name = value(&mut arguments, "--target")?;
                options.target = Some(
                    CodecTarget::parse(&name)
                        .ok_or_else(|| format!("unknown codec target {name:?}"))?,
                );
            }
            "--case" => {
                options.case = Some(
                    value(&mut arguments, "--case")?
                        .parse()
                        .map_err(|_| "--case must be a non-negative integer".to_owned())?,
                );
            }
            "--progress" => {
                options.progress = Some(PathBuf::from(value(&mut arguments, "--progress")?));
            }
            "--artifact" => {
                options.artifact = Some(PathBuf::from(value(&mut arguments, "--artifact")?));
            }
            "--replay" => {
                options.replay = Some(PathBuf::from(value(&mut arguments, "--replay")?));
            }
            "-h" | "--help" => return Err(usage().into()),
            _ => return Err(format!("unknown argument {argument:?}")),
        }
    }
    if options.replay.is_some()
        && (options.target.is_some()
            || options.case.is_some()
            || options.progress.is_some()
            || options.artifact.is_some()
            || options.runs != DEFAULT_RUNS)
    {
        return Err("--replay cannot be combined with generation options".into());
    }
    if options.case.is_some() && options.target.is_none() {
        return Err("--case requires --target".into());
    }
    Ok(options)
}

fn record_progress(path: Option<&PathBuf>, target: CodecTarget, case: u64) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    std::fs::write(
        path,
        format!(
            "seed_schema={SEED_SCHEMA_VERSION}\ntarget={}\ncase={case}\n",
            target.name()
        ),
    )
    .map_err(|error| format!("write progress {}: {error}", path.display()))
}

fn execute(options: &Options) -> Result<(), String> {
    if let Some(path) = &options.replay {
        let artifact = read_artifact(path)?;
        exercise(artifact.target, &artifact.input)?;
        println!(
            "rust-codec-fuzz replay=PASS target={} case={} bytes={}",
            artifact.target,
            artifact.case,
            artifact.input.len()
        );
        return Ok(());
    }

    let targets: Vec<_> = options
        .target
        .map(|target| vec![target])
        .unwrap_or_else(|| CodecTarget::ALL.to_vec());
    let mut executed = 0u64;
    for target in targets {
        let cases: Box<dyn Iterator<Item = u64>> = match options.case {
            Some(case) => Box::new(std::iter::once(case)),
            None => Box::new(0..options.runs),
        };
        for case in cases {
            record_progress(options.progress.as_ref(), target, case)?;
            let result = run_case(target, case);
            let artifact = match result {
                Ok(artifact) => artifact,
                Err(error) => {
                    if let Some(path) = &options.artifact {
                        let preserve =
                            afsplus_format_fuzz::mutated_input(target, case).and_then(|input| {
                                write_artifact(
                                    path,
                                    &FuzzArtifact {
                                        target,
                                        case,
                                        input,
                                    },
                                )
                            });
                        if let Err(preserve_error) = preserve {
                            return Err(format!(
                                "target={target} case={case}: {error}; artifact preservation failed: {preserve_error}"
                            ));
                        }
                    }
                    return Err(format!("target={target} case={case}: {error}"));
                }
            };
            if options.case.is_some() {
                if let Some(path) = &options.artifact {
                    write_artifact(path, &artifact)?;
                }
            }
            executed += 1;
        }
    }
    println!(
        "rust-codec-fuzz result=PASS targets={} cases={} seed-schema={SEED_SCHEMA_VERSION}",
        options.target.map_or(CodecTarget::ALL.len(), |_| 1),
        executed
    );
    Ok(())
}

fn main() -> ExitCode {
    let options = match parse() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("afsplus-format-fuzz: {error}\n{}", usage());
            return ExitCode::from(2);
        }
    };
    match execute(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("afsplus-format-fuzz: {error}");
            ExitCode::FAILURE
        }
    }
}
