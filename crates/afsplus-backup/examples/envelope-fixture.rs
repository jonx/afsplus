//! Emit an ordinary-file envelope for independent verification.
use afsplus_backup::{envelope, tar};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdout = std::io::stdout();
    let mut writer = envelope::Writer::new(
        stdout.lock(),
        tar::Limits {
            members: 1,
            member_bytes: 5,
            trailing_zero_blocks: 0,
        },
    )?;
    writer.start(
        &tar::Header {
            path: "files/folder/payload".into(),
            link: String::new(),
            kind: tar::Kind::File,
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
    let (_output, _receipt) = writer.finish()?;
    Ok(())
}
