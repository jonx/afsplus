# 09. Feature Framework

## 1. Objective

AFS+ must evolve without requiring every implementation to support every future feature.

The design separates:

- format features recorded on disk
- loadable implementation providers
- compatibility profiles

These are different concepts.

## 2. Feature identity

Features use stable string IDs with reverse-DNS style ownership, for example:

```text
org.aros.afsplus:catalog
org.aros.afsplus:change-stream
org.aros.afsplus:inline-data
```

The on-disk representation may use compact numeric IDs for standardized core features, but tools must retain a stable globally unique textual identity.

## 3. Feature classes

### COMPAT

An implementation that does not understand the feature may still mount read/write safely.

Example candidate: a fully derived catalog whose staleness can be detected by core generations.

### RO_COMPAT

An implementation that does not understand the feature may mount read-only safely.

### INCOMPAT

The implementation must understand the feature before mounting.

## 4. Feature states

AFS+ adopts a useful distinction between:

- disabled
- enabled but unused
- active

An enabled feature that has never changed authoritative on-disk structures should not unnecessarily lock out older implementations.

## 5. Dependencies

Feature metadata declares dependencies.

Enabling a feature must verify its dependencies before any incompatible on-disk state is created.

## 6. Compatibility profiles

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

## 7. Loadable providers

Only narrow feature categories should be pluggable at runtime.

Examples:

- compression codec
- checksum algorithm
- encryption transform

Core directory, object, extent, and allocation semantics are not runtime-pluggable.

This prevents a Reiser4-style explosion of mutually incompatible fundamental layouts.
