//! Emit a harmless regular-file archive for independent-reader qualification.
use afsplus_backup::tar::{Header, Kind, Limits, Writer};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdout = std::io::stdout();
    let mut writer = Writer::new(
        stdout.lock(),
        Limits {
            members: 1,
            member_bytes: 5,
            trailing_zero_blocks: 0,
        },
    );
    writer.start(
        &Header {
            path: "folder/payload".into(),
            link: String::new(),
            kind: Kind::File,
            mode: 0o640,
            uid: 12,
            gid: 34,
            size: 5,
            mtime: 123,
            uname: "user".into(),
            gname: "group".into(),
        },
        None,
    )?;
    writer.write_payload(b"hello")?;
    let _output = writer.finish()?;
    Ok(())
}
