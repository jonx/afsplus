# 09. Feature Framework

## 1. Objective

AFS+ must evolve without requiring every implementation to support every future feature forever.

The design separates:

- format features recorded on disk
- implementation support for those features
- loadable implementation providers
- compatibility profiles

These are different concepts.

A feature may be useful experimentally and later prove unnecessary. The format must allow new implementations to stop creating or activating such a feature without making existing volumes unreadable or reusing its identity for something else.

## 2. Feature identity

Features use stable string IDs with reverse-DNS style ownership, for example:

```text
org.aros.afsplus:catalog
org.aros.afsplus:change-stream
org.aros.afsplus:inline-data
```

The on-disk representation may use compact numeric IDs for standardized core features, but tools must retain a stable globally unique textual identity.

The current prototype identification block carries compact `COMPAT`,
`RO_COMPAT`, and `INCOMPAT` summaries. The first assigned identity is:

```text
INCOMPAT bit 0 = org.aros.afsplus:intent-log
```

This summary is executable at mount; it does not replace future registry
records for dependencies, lifecycle state, or feature parameters.

Once allocated, a feature identity is never reused for a different semantic feature, even if the original feature is later deprecated or retired.

## 3. Feature classes

### COMPAT

An implementation that does not understand the feature may still mount read/write safely.

Example candidate: a non-authoritative catalog whose staleness can be detected by core generations.

### RO_COMPAT

An implementation that does not understand the feature may mount read-only safely.

### INCOMPAT

The implementation must understand the feature before mounting read/write or read-only according to the feature contract.

## 4. On-disk feature states

AFS+ distinguishes:

- disabled
- enabled but unused
- active

An enabled feature that has never changed authoritative on-disk structures should not unnecessarily lock out older implementations.

A feature flag may be cleared only when the filesystem no longer contains any state whose interpretation or correctness depends on that feature. If conversion is required, conversion must complete and commit before the active state is removed.

## 5. Implementation lifecycle

Implementation/support lifecycle is separate from the on-disk state. Registry metadata may classify a feature as:

- experimental
- stable
- deprecated
- retired

`deprecated` means existing volumes remain supported, but new volumes should not enable the feature by default.

`retired` means new implementations are not expected to create new active instances of the feature. Existing active volumes must still be handled according to the feature compatibility class, either by retained legacy support, a compatibility module/tool, or an explicit migration path. They must never be silently misinterpreted.

A retired feature ID remains permanently reserved.

## 6. Authoritative, rebuildable, and discardable are different properties

The registry must not use one `derived` boolean to mean several different things.

Features may separately declare:

- `authoritative`: filesystem correctness depends on this state
- `rebuildable`: equivalent state can be reconstructed from authoritative current filesystem state
- `discardable`: the state may be dropped without corrupting the filesystem, although consumers may lose history or performance

Examples:

- global catalog: non-authoritative, rebuildable, discardable
- persistent change stream: non-authoritative, **not rebuildable as history**, discardable with `RESCAN_REQUIRED`
- xattrs: authoritative when active

This distinction is part of the repair contract.

## 7. Dependencies

Feature metadata declares dependencies.

Enabling a feature must verify its dependencies before any incompatible on-disk state is created.

Removing/deactivating a feature must also verify that no active dependent feature still requires it.

## 8. Compatibility profiles

A profile is a named allowed-feature set.

Examples:

### reader-minimal

Readable by tiny and recovery implementations.

### classic-rw

Restricts the volume to features expected to be practical on older Amiga-family systems.

### boot-safe

Restricts the volume to features understood by boot/recovery components.

### workstation

Default modern AROS profile.

### full

Allows all standardized stable features supported by the implementation.

Profiles are policy. They do not create alternate filesystem formats.

## 9. Loadable providers

Only narrow feature categories should be pluggable at runtime.

Examples:

- compression codec
- checksum algorithm
- encryption transform

Core directory, object, extent, and allocation semantics are not runtime-pluggable.

This prevents a Reiser4-style explosion of mutually incompatible fundamental layouts.
