# Getting started with AFS+

One page, from nothing to an AFS+ volume running under AROS: get it, build
it, make a volume, break it, run it, and where to read next.

## 1. What it is, in five lines

AFS+ is a file system for AROS whose every change is atomic: copy-on-write
blocks, published by one checkpoint write into one of two slots. An operation
exists whole or not at all, so there is no validator and nothing to validate
after a power cut. On AROS it is an ordinary DOS handler: packets, a
DOSDriver, `FileSystem.resource`, `Buffers`, notifications, links, protection
bits, comments. What is new (clones, watches, extended attributes, a health
report) goes through one extension packet that an old program never meets.

## 2. Get it and build it

You need `git` and stable Rust ([rustup.rs](https://rustup.rs)). Nothing else
for the host side: Linux, macOS or Windows.

```sh
git clone https://github.com/jonx/afsplus.git && cd afsplus
cargo build --workspace --release
```

The programs land in `target/release`. Tests are run by name, never as a
suite ([development-method](implementation/development-method.md)):

```sh
cargo test -p afsplus-check --test crash_replay
```

## 3. Your first volume, without mounting anything

```sh
cd target/release
./mkafsplus --size-mib 64 --label Work work.img     # format
./afsplus-populate work.img ~/some/drawer           # copy a drawer in
./afsplus-info work.img                             # what the volume says
./afsplus-check work.img                            # independent checker: exit 0 is clean
./afsplus-explain work.img path /some/file          # what the image believes, and why
./afsplus-extract work.img out/                     # recover every object and a manifest, read-only
```

Every tool but `mkafsplus` and `afsplus-populate` opens the image read-only,
and every one speaks `--json` ([tools-spec](tools/tools-spec.md)).

## 4. Mount it on the host (optional)

```sh
cargo build --release -p afsplus-fuse --features fuser-adapter --bin afsplus-mount
mkdir /tmp/work && target/release/afsplus-mount work.img /tmp/work
```

Linux needs FUSE; macOS needs macFUSE 5.4.0 or later and
`--features macfuse-mount` ([mounted-volume-testing](testing/mounted-volume-testing.md)).

## 5. Try to break it

This is the point of the design, so do it early. Copy files into a mounted
volume and kill the mount, or truncate a copy of the image at any byte, then:

```sh
./afsplus-check work.img && ./afsplus-info work.img
```

The volume mounts at its last checkpoint plus what was fsynced; the checker
stays clean. `afsplus-image-diff before.img after.img` says what two images
differ by in file-system terms. If you find an image the checker calls clean
and a mount refuses, or the reverse, that image is the bug report.

## 6. Run it under AROS

AFS+ ships outside the AROS tree: a handler in `L:`, a DOSDriver in
`DEVS:DOSDrivers`, a disk or an image behind any block device.

| You have | Do |
|---|---|
| A hosted AROS build (`darwin-aarch64` is the qualified one) and the AROS cross toolchain | `tools/package-aros-alpha0.sh` builds the handler, the tools and a checked image into one directory; [aros-alpha0-package](docs/aros-alpha0-package.md) says where each file goes (`L:afsplus-handler`, `DEVS:DOSDrivers/AFSPLUS19`, `C:AFSPlusInfo`...) |
| The files installed | `Mount AFSPLUS19:` then use it as any volume; `AFSPlusInfo AFSPLUS19:` reports the volume, `AFSPlusTour AFSPLUS19:` shows a clone, a watch and an attribute in ten seconds |
| A wish to boot from it | `afsplus-disk wrap disk.img work.img` writes a GPT disk whose partition AROS recognises (DosType `AFS+`, [ADR-122](adr/ADR-122-aros-partition-identity.md)); the handler as a boot module does the rest ([native-preparation](implementation/native-preparation.md)) |
| A new disk driver | `AFSPlusDriverProbe` checks it keeps the promises AFS+ needs ([block-device contract](docs/aros-block-device-contract.md)) |

The DOSDriver is a normal mountlist. `Buffers` sizes the read cache as for
FFS; the `Control` string takes `COMMIT=SYNC` (every operation durable when
it returns) or `COMMIT=<seconds>` (the default, 5: changes gather and commit
together, and a crash loses the last seconds whole, never half an operation),
and `CACHE=AUTO` ([aros-native-bridge](docs/aros-native-bridge.md)). A
program that needs its bytes on the disk calls `Flush()`, as everywhere.

The gates that prove all this on a hosted AROS are one script each, listed
in [tools/README](tools/README.md): DOS semantics, boot from AFS+ (S2), 24
power cuts in a row with nothing torn (S3), the benchmark against FFS.

## 7. Where to read, in this order

1. [README](README.md): why it exists, how it works, where it stands.
2. [24-filesystem-comparison](docs/24-filesystem-comparison.md) and
   [22-pfs3-and-pfs4-lessons](docs/22-pfs3-and-pfs4-lessons.md): AFS+ beside
   FFS, SFS and PFS, and what it took from them.
3. [08-transactions-and-journal](docs/08-transactions-and-journal.md) and
   [19-recovery-and-maintenance](docs/19-recovery-and-maintenance.md): why a
   cut cannot tear anything.
4. [16-classic-systems](docs/16-classic-systems.md) and
   [aros-native-bridge](docs/aros-native-bridge.md): the AROS side, packet by
   packet.
5. [13-filesystem-api-v2](docs/13-filesystem-api-v2.md): what a new program
   can ask that an old file system cannot answer.
6. [03-on-disk-format](docs/03-on-disk-format.md) and
   [18-third-party-integration](docs/18-third-party-integration.md): the
   format, and the small C probe for other people's tools.
7. [ROADMAP](ROADMAP.md) and [performance-program](implementation/performance-program.md):
   what is open and what is being made faster.

Decisions are in [adr/](adr/README.md), one file each, with their reasons.

## 8. Telling us what you found

An issue on the repository with: what you did, what you expected, the
output of `afsplus-check --json` and `afsplus-info --json`, and the image if
it is small enough. On AROS add the output of `AFSPlusInfo <volume>:`. A
file system is judged by the worst thing anyone managed to do to it, so the
unkind tests are the useful ones.
