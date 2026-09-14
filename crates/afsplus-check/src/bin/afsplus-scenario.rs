//! Private memory scenario runner. Fixed framed stdout; no filesystem output paths.
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::{
    replay_trace::{base_digest, Limits, Trace},
    scenario::{inspect, Plan},
};
use std::io::{self, Read, Write};

fn hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        write!(&mut out, "{byte:02x}").expect("string write");
    }
    if out.is_empty() {
        "-".into()
    } else {
        out
    }
}
fn header(out: &mut impl Write, name: &str, length: usize) -> io::Result<()> {
    out.write_all(&(name.len() as u16).to_le_bytes())?;
    out.write_all(name.as_bytes())?;
    out.write_all(&(length as u64).to_le_bytes())
}
fn artifact(out: &mut impl Write, name: &str, data: &[u8]) -> io::Result<()> {
    header(out, name, data.len())?;
    out.write_all(data)
}
fn image(out: &mut impl Write, name: &str, device: &mut MemoryBackend) -> Result<(), String> {
    header(
        out,
        name,
        device.total_blocks() as usize * device.block_size(),
    )
    .map_err(|e| e.to_string())?;
    let mut block = vec![0; device.block_size()];
    for lba in 0..device.total_blocks() {
        device
            .read_block(lba, &mut block)
            .map_err(|e| e.to_string())?;
        out.write_all(&block).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn run() -> Result<(), String> {
    if std::env::args_os().len() != 1 {
        return Err("scenario runner accepts only stdin".into());
    }
    let mut input = Vec::new();
    io::stdin()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    let (fault, commands) = if input.starts_with(b"AFSCUT01 ") {
        let split = input
            .iter()
            .position(|&b| b == b'\n')
            .ok_or("cut header terminator")?;
        let header = std::str::from_utf8(&input[..split]).map_err(|_| "cut header encoding")?;
        let fields: Vec<_> = header.split(' ').collect();
        let ["AFSCUT01", operation, offset, variant] = fields.as_slice() else {
            return Err("cut header fields".into());
        };
        let number = |text: &str, maximum: usize| -> Result<usize, String> {
            if text.is_empty()
                || !text.bytes().all(|b| b.is_ascii_digit())
                || (text.len() > 1 && text.starts_with('0'))
            {
                return Err("cut integer encoding".into());
            }
            let value = text.parse::<usize>().map_err(|_| "cut integer")?;
            if value > maximum {
                return Err("cut integer admission".into());
            }
            Ok(value)
        };
        (
            Some((
                number(operation, 1023)?,
                number(offset, 65536)?,
                number(variant, 4131)?,
            )),
            &input[split + 1..],
        )
    } else {
        (None, input.as_slice())
    };
    let mut run = Plan::parse(commands)?.run()?;
    if let Some((operation, offset, variant)) = fault {
        if run.failure.is_some() {
            return Err("fault scenario did not finish recording".into());
        }
        let event = run
            .events
            .get(operation)
            .ok_or("fault operation outside scenario")?;
        let cut = event
            .first_block_operation
            .checked_add(offset)
            .ok_or("fault cut overflow")?;
        if cut > event.end_block_operation {
            return Err("fault offset outside operation".into());
        }
        run.result = afsplus_check::crash_replay::select(&run.base, &run.log, cut, variant)?;
    }
    let limits = Limits {
        wire_bytes: 66 * 1024 * 1024,
        operations: 65536,
        block_size: 4096,
        base_blocks: 65536,
    };
    let trace = Trace {
        block_size: run.base.block_size(),
        blocks: run.base.total_blocks(),
        base_digest: base_digest(&mut run.base, limits)?,
        operations: run.log,
    }
    .encode(limits)?;
    let mut observed = String::from("AFSOBS01\n");
    match run.failure {
        Some((index, error)) => {
            observed.push_str(&format!("run error {index} {}\n", hex(error.as_bytes())))
        }
        None => observed.push_str("run ok\n"),
    }
    match inspect(run.result.clone(), 16 * 1024 * 1024) {
        Err(error) => observed.push_str(&format!("observe error {}\n", hex(error.as_bytes()))),
        Ok(entries) => {
            observed.push_str("observe ok\n");
            for entry in entries {
                let path = entry
                    .path
                    .iter()
                    .map(|s| hex(s.as_bytes()))
                    .collect::<Vec<_>>()
                    .join(",");
                match entry.data {
                    Some(data) => observed.push_str(&format!("file {path} {}\n", hex(&data))),
                    None => observed.push_str(&format!("directory {path}\n")),
                }
            }
        }
    }
    // Semantic flight records bind operation indices and resolved object IDs to
    // half-open successful block-log ranges. They are not transaction-internal events.
    let mut flight = b"AFSFLT01".to_vec();
    flight.extend_from_slice(&(run.events.len() as u32).to_le_bytes());
    for event in run.events {
        flight.extend_from_slice(&(event.operation as u32).to_le_bytes());
        flight.extend_from_slice(&(event.first_block_operation as u64).to_le_bytes());
        flight.extend_from_slice(&(event.end_block_operation as u64).to_le_bytes());
        flight.extend_from_slice(&event.object_id.to_le_bytes());
        flight.push(u8::from(event.success));
    }
    let mut out = io::BufWriter::new(io::stdout().lock());
    out.write_all(b"AFSRUN01").map_err(|e| e.to_string())?;
    artifact(&mut out, "actual.wire", observed.as_bytes()).map_err(|e| e.to_string())?;
    artifact(&mut out, "flight-recorder.bin", &flight).map_err(|e| e.to_string())?;
    artifact(&mut out, "block-io.afstrace", &trace).map_err(|e| e.to_string())?;
    image(&mut out, "start.img", &mut run.base)?;
    image(&mut out, "result.img", &mut run.result)?;
    out.flush().map_err(|e| e.to_string())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("scenario refused: {error}");
        std::process::exit(1);
    }
}
