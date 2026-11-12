# macFUSE FSKit activation on macOS

> **ADRs:** [ADR-040](../adr/ADR-040-fuse-protocol-boundary.md), [ADR-137](../adr/ADR-137-host-mount-delayed-commit-option.md) · **Spec:** none ·
> **Tests:** [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md) · **Milestones:** M08

AFS+ uses macFUSE's user-space FSKit backend on current macOS releases. It
does not require the legacy macFUSE kernel extension, reduced security, or a
restart into Recovery.

<!-- toc -->

- [Normal activation](#normal-activation)
- [Why macFUSE and not FUSE-T](#why-macfuse-and-not-fuse-t)
- [After installing or upgrading macFUSE](#after-installing-or-upgrading-macfuse)
- [Diagnose without changing the system](#diagnose-without-changing-the-system)
- [Work around an inert Settings switch](#work-around-an-inert-settings-switch)
- [Host fsync fallback](#host-fsync-fallback)
- [Restore the previous FSKit list](#restore-the-previous-fskit-list)
- [Scope and caution](#scope-and-caution)

<!-- /toc -->

## Normal activation

Install macFUSE, then open **System Settings > General > Login Items &
Extensions > File System Extensions** and enable both macFUSE modules. Prefer
this supported path whenever the switches work.

The switches can remain visually off and ignore clicks on some macOS builds.
This was reproduced on macOS 26.6.2 (25G83) with macFUSE 5.3.3. Registration
with PluginKit is not sufficient in that state: FSKit keeps its own per-user
enabled-module list.

The inert-toggle behavior and the reversible workaround below are reported
upstream in [macfuse/macfuse#1194](https://github.com/macfuse/macfuse/issues/1194).
Check that issue before using the workaround: a supported fix or activation
procedure may supersede it.

## Why macFUSE and not FUSE-T

`afsplus-mount` speaks to macFUSE's FSKit backend through its message channel
(`MFMount.framework`, loaded at run time by `afsplus-macfuse-sys`); that is
the only macOS backend the code has. FUSE-T translates FUSE into an NFS
server driven by its own libfuse loop: a session reached `INIT`, `STATFS` and
`GETATTR` and was then closed, so it would need an adapter of its own
([ADR-040](../adr/ADR-040-fuse-protocol-boundary.md)). macFUSE's FSKit
backend needs no kernel extension, no reduced security and no Recovery boot.

## After installing or upgrading macFUSE

Observed on the M5 with macFUSE 5.5.0 after `brew upgrade --cask macfuse`
(the upgrade asks for an administrator password):

1. The first mount fails with `File system extension not found`, and
   macFUSE shows a **Register File System Extension** dialog. Register it.
2. Restart the Mac. Until then the extension stays unavailable.
3. After the restart the mount fails with `File system extension not
   enabled`. Switch macFUSE on in **System Settings > General > Login Items &
   Extensions > File System Extensions**, or for the current user:

   ```sh
   pluginkit -e use -i io.macfuse.app.fsmodule.macfuse
   tools/macos-fskit-modules.sh check
   ```

4. Mounting then works.

This is the FSKit backend, a user-space extension. Homebrew's caveat about
allowing a kernel extension is generic to the cask and does not apply. If
macOS ever asks to boot into Recovery or to lower the security policy for
AFS+, the answer is no: that path is the legacy kernel extension, which AFS+
does not use.

With macFUSE 5.4.0 on macOS 26.6.2 an upgrade left the module registered but
disabled, and the `pluginkit` command alone was enough, with no restart.

## Diagnose without changing the system

From the repository root:

```sh
tools/macos-fskit-modules.sh check
```

The command reports the macOS build and macFUSE package version, verifies that
PluginKit sees both installed module identifiers, then checks their activation
in:

```text
~/Library/Group Containers/group.com.apple.fskit.settings/enabledModules.plist
```

It never requests administrator privileges.

## Work around an inert Settings switch

Use this only when both macFUSE modules are registered but the Settings
switches do not change:

```sh
tools/macos-fskit-modules.sh enable
```

The script:

1. validates the existing FSKit plist rather than creating a replacement;
2. refuses to proceed unless PluginKit sees both installed macFUSE modules;
3. makes a timestamped backup next to it;
4. adds only missing macFUSE module identifiers to a temporary copy;
5. validates and installs that copy;
6. requests one administrator authorization to restart `fskitd`; and
7. restarts the per-user FSKit services.

If authorization is cancelled or `fskitd` cannot be restarted, the script
automatically restores the module list that was current before the command.

If both identifiers are already enabled, `enable` exits without changing
anything, restarting services, or requesting administrator authorization.
The Settings switches may still look off after the workaround. Runtime
activation is authoritative: a successful AFS+ host qualification proves that
the FSKit extension can actually launch and exchange FUSE requests.

Run the qualification with:

```sh
AFSPLUS_FUSE_MOUNT_TEST=1 \
  cargo test -p afsplus-fuse --all-features --test host_mount -- --ignored --nocapture
```

The test mounts a fresh image below `/Volumes`, runs the Alpha-0 operation
matrix, unmounts it, and requires a clean AFS+ checker result. Once activation
has succeeded, this mount workflow does not request administrator privileges.

## Host fsync fallback

The macFUSE FSKit transport flushes dirty pages with FUSE `WRITE` and returns
success from host `fsync(2)` and `F_FULLFSYNC` without delivering a FUSE
`FSYNC` request (checked again with macFUSE 5.5.0). The
AFS+ macOS mount therefore uses durable data replies: each `WRITE` and
size-changing `SETATTR` completes the bounded AFS+ durability operation before
the transport receives success. This is scoped to the FSKit mount; the
host-neutral FUSE protocol and AROS adapters retain deferred writeback and an
explicit `fsync` durability point.

The fallback requires neither the legacy kernel extension nor administrator
authorization. It trades batching for correctness and can add up to the normal
data-and-record barriers to each delivered write. Host benchmark reports must
identify this mode rather than comparing it silently with an ordinary
writeback filesystem. `--commit=delayed` gives the fallback up on request
for populating an image, where the unmount is the durability point
([image-workflow](../testing/image-workflow.md#populating-an-image-with-many-files),
[ADR-137](../adr/ADR-137-host-mount-delayed-commit-option.md)).

## Restore the previous FSKit list

The activation command prints its exact backup path. Restore it with:

```sh
tools/macos-fskit-modules.sh restore \
  "$HOME/Library/Group Containers/group.com.apple.fskit.settings/enabledModules.plist.afsplus-backup-YYYYMMDD-HHMMSS"
```

Restore also preserves the current plist in a new timestamped backup before
replacing it and requests one administrator authorization to restart `fskitd`.

## Scope and caution

`enabledModules.plist` is an implementation detail of macOS, not a public
Apple administration interface. Keep the OS build and macFUSE version in bug
reports, retain the generated backup, and retry the normal Settings path after
system updates. Do not copy another machine's entire plist: module lists may
differ between macOS versions.

The script is BSD-2-Clause and may be shared independently. When posting it,
describe it as an unofficial, reversible workaround verified on macOS 26.6.2
(25G83) with macFUSE 5.3.3, not as a supported macFUSE or Apple procedure.
