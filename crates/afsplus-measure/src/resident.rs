//! Host-only phase-boundary RSS snapshots. No unsafe OS ABI or filesystem code.
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

static ROUNDS: AtomicUsize = AtomicUsize::new(0);

pub fn rounds() -> usize {
    ROUNDS.load(Ordering::Relaxed)
}

pub fn configure(count: usize) -> Result<(), String> {
    if !(3..=32).contains(&count) {
        return Err("resident rounds must be 3..=32".into());
    }
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Err("resident provider supports macOS/Linux ps units only".into());
    }
    probe()?;
    ROUNDS.store(count, Ordering::Relaxed);
    Ok(())
}

fn parse(bytes: &[u8]) -> Result<u64, String> {
    if bytes.len() > 128 {
        return Err("resident provider output limit".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "resident provider encoding")?;
    let number = text.trim_matches(|c: char| c.is_ascii_whitespace());
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err("resident provider must return one unsigned integer".into());
    }
    let kib: u64 = number
        .parse()
        .map_err(|_| "resident provider integer overflow")?;
    if kib == 0 {
        return Err("resident provider returned zero for the live workload".into());
    }
    kib.checked_mul(1024)
        .ok_or_else(|| "resident byte count overflow".into())
}

fn probe() -> Result<u64, String> {
    // The still-running caller selects only itself. No PID discovery, shell,
    // PATH search, environment capture, or polling of a potentially reused PID.
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("resident provider: {e}"))?;
    if !output.status.success() {
        return Err(format!("resident provider failed: {}", output.status));
    }
    parse(&output.stdout)
}

#[derive(Clone, Copy)]
pub struct Snapshot {
    pub bytes: u64,
    pub probe_wall_ns: u128,
}

pub fn snapshot() -> Option<Snapshot> {
    if rounds() == 0 {
        return None;
    }
    let start = Instant::now();
    let bytes = probe().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1);
    });
    Some(Snapshot {
        bytes,
        probe_wall_ns: start.elapsed().as_nanos(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_ambiguous_missing_and_overflowing_measurements() {
        assert_eq!(parse(b" 1234\n").unwrap(), 1234 * 1024);
        for bytes in [
            b"".as_slice(),
            b"0",
            b"-1",
            b"+1",
            b"1 2",
            b"1\n2",
            b"RSS\n123",
            b"18446744073709551615",
            b"18446744073709551616",
            b"\xff",
        ] {
            assert!(parse(bytes).is_err(), "{bytes:?}");
        }
        assert!(parse(&[b'1'; 129]).is_err());
    }
}
