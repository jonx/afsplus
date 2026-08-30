# ADR-049: Qualify the Hosted desktop session after the AFS+ system pivot

Status: Accepted for Hosted S1b

## Context

S1a proved that the six core assigns and target commands could move to AFS+
after bootstrap. It deliberately did not prove that the normal GUI stack,
preferences and representative applications could load a much larger system
tree through the native handler.

A valid S1b result needs runtime evidence. Merely placing binaries in an image
does not prove that Wanderer or an application loaded, remained alive, displayed
content or wrote persistent state.

## Decision

`tools/build-aros-s1-image.sh` provides a separate `desktop` profile. It creates
a 256 MiB image containing the core S1 payload plus the named MacAROS build's
`Classes`, `Devs`, `Fonts`, `Locale`, `Prefs`, `System`, `Tools`, `Utilities`
and `L` trees. It adds the exact desktop-sequence commands, a deterministic
backdrop and `AFSPlusS1bProbe`; `content-SHA256SUMS` handles names containing
spaces without splitting them.

After the S1a pivot, the S1b sequence establishes `ENV:`, `ENVARC:`, `LOCALE:`,
`FONTS:`, `WANDERER:`, `THEMES:`, `THEME:` and `IMAGES:` on AFS+, extends
`LIBS:` with `Classes:`, initializes datatypes, audio modes and IPrefs, then
launches Wanderer, Locale Preferences and Clock. The target probe verifies the
assigns with `SameLock`, checks the three executables and theme data, and
durably writes an `ENVARC:` marker. `SetEnv SAVE` provides a second persistent
configuration write.

The Hosted gate requires:

- both target probes and the bootstrap pivot to report PASS;
- the durable marker, saved environment value and runtime provenance copied
  back through `MacRW:` with exact content;
- a kernel task dump containing live Wanderer, IPrefs, Locale and Clock code;
- a captured framebuffer with a non-empty foreground region;
- no AFS+, trap or alert failure in the Hosted log; and
- a clean strict checker with no pending intent-log record after shutdown.

## Evidence

The Hosted S1b gate passed on 2026-08-30:

- 581 directories, 2,826 files and 50,384,697 payload bytes;
- image generation 3,408, 3,408 objects and 14,185 data blocks before boot;
- `AFSPlusS1Probe`, `AFSPlusS1bProbe` and `AFSPlusS1Pivot` all passed;
- live task stacks identified Wanderer, IPrefs, Locale and Clock;
- the framebuffer evidence contained 2,036 non-background pixels in the
  bounded desktop region;
- both durable preference values read back as `afsplus-s1b`; and
- generation 3,414, 3,411 objects, 14,188 data blocks, a clean checker and zero
  pending intent-log records after execution.

The gate imports every payload through the portable transactional core. The
3,408 pre-boot generation therefore also exercises repeated COW publication and
reclamation over a realistic many-file system tree.

## Boundaries

This accepts the statement "a normal Hosted MacAROS session runs on AFS+ after
bootstrap." It does not claim autonomous AFS+ boot selection, native Apple
Silicon execution or classic m68k qualification. The S1a bootstrap dependencies
and retained `BOOTSYS:` emergency assign remain explicit.

The prototype comparison key is still identity and therefore case-sensitive.
The stock desktop sequence spells the theme directory `Images`, while the
manifested tree contains `images`; S1b uses the exact stored spelling. ADR-008
still requires case-insensitive, spelling-preserving lookup for traditional
AROS/Amiga namespaces before epoch 1. This gate does not waive that contract.

AmigaDOS `If` and `EndIf` are files in `C:`, not shell built-ins. They are part
of the desktop payload so post-pivot control flow never falls back to the old
system tree.

## Consequences

Hosted S1 is complete. The next integration work is handler lifecycle and an
in-tree MacAROS build, followed by the same-image contract on native MacAROS,
then m68k emulation and the physical A500.
