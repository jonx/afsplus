# AFS+ Hosted MacAROS S1 image

This directory is produced by `tools/build-aros-s1-image.sh`. Its `profile.txt`
identifies either the 64 MiB `core` S1a image or the 256 MiB `desktop` S1b
image. Neither is an autonomous boot volume: both are entered through the
post-bootstrap `SYS:` pivot.

The image contains:

- `C:` commands needed to execute and inspect the pivot;
- the target-only `AFSPlusS1Probe`;
- the complete `Libs:` directory from the named MacAROS build;
- empty `L:` and `Devs:` roots so their system assigns can move atomically;
- `S:S1-Sequence`, which is executed only after the pivot; and
- the fixed `SYS:s1-origin` provenance marker.

The `desktop` profile additionally contains `Classes:`, `Devs:`, `Fonts:`,
`Locale:`, `Prefs:`, `System:`, `Tools:` and `Utilities:` from the named build,
the commands used by the normal MacAROS desktop sequence, a deterministic
backdrop, and the `AFSPlusS1bProbe`. It starts Wanderer, IPrefs, Locale and Clock
only after their assigns and paths point to AFS+.

The current prototype comparison key is case-sensitive, so the S1b sequence
uses the exact source-tree spelling `THEME:images`. This is a recorded prototype
boundary, not the final Amiga namespace contract; the pinned epoch-1 comparison
key must provide the selected case-insensitive policy before format freeze.

`content-SHA256SUMS` manifests every payload file by its image-relative path.
The source MacAROS commit/build identity belongs in the gate result alongside
this manifest. Alpha-0 does not yet preserve POSIX mode, extended attributes or
symbolic links; S1 uses native AROS binaries, whose loading does not depend on a
POSIX executable bit.

The old system tree remains an explicit bootstrap dependency for the running
Shell, `fdsk.device`, the AFS+ handler, the initial Startup-Sequence, and the
already-loaded `AFSPlusS1Pivot`. That helper installs all new assigns without
requiring the old Shell to read another script line, then executes
`S:S1-Sequence` from AFS+. The first S1 gate deliberately retains `BOOTSYS:` as
a recorded emergency/bootstrap alias: removing an assign while AROS releases
the old handler lock needs its own lifecycle gate. The post-pivot sequence does
not use that alias, and the probe verifies that it is a distinct volume.

The S1b gate also captures a populated framebuffer, a kernel task dump and a
durable `ENVARC:` marker. These prove a live desktop and representative GUI
programs rather than merely the presence of their binaries in the image.
