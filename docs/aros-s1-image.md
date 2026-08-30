# AFS+ Hosted MacAROS S1 image

This directory is produced by `tools/build-aros-s1-image.sh`. `Unit19.s1` is a
64 MiB AFS+ image containing the first manifested system subset for the
post-bootstrap `SYS:` pivot. It is not a bootable or complete AROS distribution.

The image contains:

- `C:` commands needed to execute and inspect the pivot;
- the target-only `AFSPlusS1Probe`;
- the complete `Libs:` directory from the named MacAROS build;
- empty `L:` and `Devs:` roots so their system assigns can move atomically;
- `S:S1-Sequence`, which is executed only after the pivot; and
- the fixed `SYS:s1-origin` provenance marker.

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
