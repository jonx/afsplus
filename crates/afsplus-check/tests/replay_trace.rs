use afsplus_block::powercut::for_each_overlay_crash_state;
use afsplus_block::{
    crash_states, BlockDevice, MemoryBackend, OverlayBackend, OverlayLimits, RecordedOp,
    TraceBackend,
};
use afsplus_check::replay_trace::{base_digest, Limits, Trace};
use sha2::{Digest, Sha256};

fn limits() -> Limits {
    Limits {
        wire_bytes: 16384,
        operations: 32,
        block_size: 4096,
        base_blocks: 16,
    }
}
fn fixture() -> (MemoryBackend, Trace) {
    let mut base = MemoryBackend::new(512, 8);
    base.write_block(1, &[7; 512]).unwrap();
    let digest = base_digest(&mut base, limits()).unwrap();
    (
        base,
        Trace {
            block_size: 512,
            blocks: 8,
            base_digest: digest,
            operations: vec![
                RecordedOp::Write {
                    lba: 1,
                    data: vec![9; 512],
                },
                RecordedOp::Flush,
                RecordedOp::Write {
                    lba: 1,
                    data: vec![11; 512],
                },
                RecordedOp::Write {
                    lba: 2,
                    data: vec![13; 512],
                },
            ],
        },
    )
}
fn reseal(wire: &mut [u8]) {
    let end = wire.len() - 32;
    let digest = Sha256::digest(&wire[..end]);
    wire[end..].copy_from_slice(&digest);
}

#[test]
fn decoded_trace_replays_exact_oracle_states() {
    let (base, trace) = fixture();
    let wire = trace.encode(limits()).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "afsplus-trace-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let artifact = directory.join("block-io.afstrace");
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&artifact)
            .unwrap();
        file.write_all(&wire).unwrap();
        file.sync_all().unwrap();
    }
    let persisted = std::fs::read(&artifact).unwrap();
    std::fs::remove_file(&artifact).unwrap();
    std::fs::remove_dir(&directory).unwrap();
    let decoded = Trace::decode(&persisted, limits()).unwrap();
    assert_eq!(decoded.encode(limits()).unwrap(), wire);
    let mut checked = TraceBackend::new(base.clone());
    decoded.verify_base(&mut checked, limits()).unwrap();
    assert_eq!(
        (
            checked.stats().reads,
            checked.stats().writes,
            checked.stats().flushes
        ),
        (8, 0, 0)
    );
    let overlay = OverlayBackend::new(
        base.clone(),
        OverlayLimits {
            branches: 3,
            entries: 16,
        },
    )
    .unwrap();
    for cut in 0..=trace.operations.len() {
        let expected = crash_states(&base, &trace.operations, cut);
        let mut count = 0;
        for_each_overlay_crash_state(
            &overlay,
            &decoded.operations,
            cut,
            |mut state, description| {
                assert_eq!(description, expected[count].description);
                for lba in 0..8 {
                    let mut bytes = [0; 512];
                    state.read_block(lba, &mut bytes)?;
                    assert_eq!(bytes.as_slice(), expected[count].image.peek(lba));
                }
                count += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(count, expected.len());
    }
}

#[test]
fn corruption_and_every_truncated_prefix_refuse_decoding() {
    let (_, trace) = fixture();
    let wire = trace.encode(limits()).unwrap();
    for end in 0..wire.len() {
        assert!(
            Trace::decode(&wire[..end], limits()).is_err(),
            "prefix {end}"
        );
    }
    for offset in 0..wire.len() {
        let mut damaged = wire.clone();
        damaged[offset] ^= 1;
        assert!(Trace::decode(&damaged, limits()).is_err(), "byte {offset}");
    }
}

#[test]
fn resealed_invalid_records_and_admission_limits_refuse() {
    let (_, trace) = fixture();
    let wire = trace.encode(limits()).unwrap();
    for (offset, bytes) in [
        (0, b"WRONGMAG".to_vec()),
        (8, 2u32.to_le_bytes().to_vec()),
        (12, 513u32.to_le_bytes().to_vec()),
        (16, 0u64.to_le_bytes().to_vec()),
        (24, u64::MAX.to_le_bytes().to_vec()),
        (64, vec![99]),
        (65, 8u64.to_le_bytes().to_vec()),
    ] {
        let mut changed = wire.clone();
        changed[offset..offset + bytes.len()].copy_from_slice(&bytes);
        reseal(&mut changed);
        assert!(
            Trace::decode(&changed, limits()).is_err(),
            "offset {offset}"
        );
    }
    let mut trailing = wire.clone();
    trailing.insert(trailing.len() - 32, 0);
    reseal(&mut trailing);
    assert!(Trace::decode(&trailing, limits()).is_err());
    for cap in [
        Limits {
            wire_bytes: wire.len() - 1,
            ..limits()
        },
        Limits {
            operations: 3,
            ..limits()
        },
        Limits {
            block_size: 256,
            ..limits()
        },
        Limits {
            base_blocks: 7,
            ..limits()
        },
    ] {
        assert!(Trace::decode(&wire, cap).is_err());
        assert!(trace.encode(cap).is_err());
    }
}

#[test]
fn mismatched_base_and_geometry_fail_without_writes() {
    let (mut base, trace) = fixture();
    base.write_block(7, &[1; 512]).unwrap();
    let mut checked = TraceBackend::new(base);
    assert!(trace.verify_base(&mut checked, limits()).is_err());
    assert_eq!((checked.stats().writes, checked.stats().flushes), (0, 0));
    let mut wrong = TraceBackend::new(MemoryBackend::new(512, 9));
    assert!(trace.verify_base(&mut wrong, limits()).is_err());
    assert_eq!(wrong.stats().reads, 0);
    let mut bounded = TraceBackend::new(MemoryBackend::new(512, 8));
    assert!(trace
        .verify_base(
            &mut bounded,
            Limits {
                base_blocks: 7,
                ..limits()
            }
        )
        .is_err());
    assert_eq!(bounded.stats().reads, 0);
}

#[test]
fn python_independently_checks_trace_and_base_digests() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let (_, trace) = fixture();
    let wire = trace.encode(limits()).unwrap();
    let mut child = Command::new("python3").args(["-c", r#"
import hashlib,struct,sys
wire=sys.stdin.buffer.read()
assert wire[:8] == b'AFSTRC00'
version,bs,blocks,count=struct.unpack('<IIQQ',wire[8:32])
assert (version,bs,blocks,count) == (1,512,8,4)
assert hashlib.sha256(wire[:-32]).digest() == wire[-32:]
base=hashlib.sha256(b'AFS+ replay base v1\0'+struct.pack('<IQ',bs,blocks))
for lba in range(blocks):
    base.update(bytes([7 if lba == 1 else 0])*bs)
assert base.digest() == wire[32:64]
position=64
operations=[]
for _ in range(count):
    tag=wire[position]; position+=1
    if tag==2:
        operations.append(('flush',))
    else:
        assert tag==1
        lba=struct.unpack('<Q',wire[position:position+8])[0];position+=8
        data=wire[position:position+bs];position+=bs
        operations.append(('write',lba,data))
assert position == len(wire)-32
assert operations == [('write',1,bytes([9])*512),('flush',),('write',1,bytes([11])*512),('write',2,bytes([13])*512)]
"#]).stdin(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(&wire).unwrap();
    assert!(child.wait().unwrap().success());
}
