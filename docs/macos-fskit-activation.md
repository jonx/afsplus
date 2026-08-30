# macFUSE FSKit activation on macOS

AFS+ uses macFUSE's user-space FSKit backend on current macOS releases. It
does not require the legacy macFUSE kernel extension, reduced security, or a
restart into Recovery.

## Normal activation

Install macFUSE, then open **System Settings > General > Login Items &
Extensions > File System Extensions** and enable both macFUSE modules. Prefer
this supported path whenever the switches work.

The switches can remain visually off and ignore clicks on some macOS builds.
This was reproduced on macOS 26.6.2 (25G83) with macFUSE 5.3.3. Registration
with PluginKit is not sufficient in that state: FSKit keeps its own per-user
enabled-module list.

## Diagnose without changing the system

From the repository root:

```sh
tools/macos-fskit-modules.sh check
```

The command verifies the two installed module identifiers in:

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
2. makes a timestamped backup next to it;
3. adds only missing macFUSE module identifiers to a temporary copy;
4. validates and installs that copy;
5. restarts the per-user FSKit services;
6. requests one administrator authorization to restart `fskitd`.

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
