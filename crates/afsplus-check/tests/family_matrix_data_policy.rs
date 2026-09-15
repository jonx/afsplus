//! In-place data policy through the family-matrix driver; see
//! tiny_cache_matrix.md. Opted-in private writes keep the ADR-062 weaker
//! contract (an older generation may hold old, new or torn-old bytes inside
//! the overwritten range); fallbacks and retained snapshots are full COW.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::volume::DataUpdatePolicy;
use afsplus_core::{CoreError, Volume};
use afsplus_format::OBJECT_ROOT;
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

const OFFSET: usize = 100;
const LENGTH: usize = 3000;
const POPULATION: usize = 400;
const NAME_LENGTH: usize = 240;

fn policy_format(variant: Variant) -> Format {
    let blocks = if variant == Variant::Eviction {
        4096
    } else {
        512
    };
    Format {
        data_policy: true,
        ..Format::new(blocks, blocks as u32)
    }
}

fn flagged<D: BlockDevice>(volume: &mut Volume<D>, id: u64, context: &str) {
    assert_eq!(
        volume.file_data_policy(id).unwrap(),
        DataUpdatePolicy::InPlacePrivate,
        "{context}"
    );
}

struct InPlace;

struct InPlaceState {
    file: u64,
    before: Vec<u8>,
    after: Vec<u8>,
    layout: (u64, u64, u64),
    entries: usize,
}

impl Family for InPlace {
    type State = InPlaceState;

    fn name(&self) -> &'static str {
        "in-place private write"
    }

    fn format(&self, variant: Variant) -> Format {
        policy_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> InPlaceState {
        let mut entries = 1;
        if variant == Variant::Eviction {
            matrix::populate(volume, "tree", POPULATION, NAME_LENGTH);
            entries += POPULATION;
        }
        let before = vec![0x11; 2 * BS];
        let file = volume.create_file_in_root("db", &before, ts(2)).unwrap();
        volume
            .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, ts(3))
            .unwrap();
        let record = volume.stat(file).unwrap().unwrap();
        let mut after = vec![0x11; 2 * BS];
        after[OFFSET..OFFSET + LENGTH].fill(0xee);
        InPlaceState {
            file,
            before,
            after,
            layout: (record.data_root, record.data_blocks, record.size_bytes),
            entries,
        }
    }

    fn captured(&self, state: &InPlaceState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, vec![0x11; 2 * BS])]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &InPlaceState,
    ) -> Result<(), CoreError> {
        volume.write_file_at(state.file, OFFSET as u64, &[0xee; LENGTH], ts(20))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &InPlaceState,
        variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(
            volume.lookup_root("db").unwrap(),
            Some(state.file),
            "{context}"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.entries,
            "{context}"
        );
        flagged(volume, state.file, context);
        let record = volume.stat(state.file).unwrap().unwrap();
        let bytes = volume.read_file(state.file).unwrap();
        let full_cow = variant == Variant::Retained;
        if delta == 1 {
            assert_eq!(bytes, state.after, "{context}: new bytes");
        } else if full_cow {
            assert_eq!(bytes, state.before, "{context}: COW keeps old bytes");
        } else {
            assert_eq!(bytes.len(), 2 * BS, "{context}");
            assert!(
                bytes[..OFFSET].iter().all(|byte| *byte == 0x11),
                "{context}"
            );
            assert!(
                bytes[OFFSET + LENGTH..].iter().all(|byte| *byte == 0x11),
                "{context}"
            );
            assert!(
                bytes[OFFSET..OFFSET + LENGTH]
                    .iter()
                    .all(|byte| *byte == 0x11 || *byte == 0xee),
                "{context}: bytes outside the old/new alphabet"
            );
        }
        if full_cow && delta == 1 {
            assert_eq!(record.size_bytes, state.layout.2, "{context}");
        } else {
            assert_eq!(
                (record.data_root, record.data_blocks, record.size_bytes),
                state.layout,
                "{context}: in-place layout"
            );
        }
    }

    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &InPlaceState,
        variant: Variant,
    ) {
        let expected = u64::from(variant != Variant::Retained);
        assert_eq!(
            volume
                .last_commit_stats()
                .unwrap()
                .data_blocks_overwritten_in_place,
            expected,
            "{variant:?}: in-place block count"
        );
    }

    /// The write updates one object-map path (root and leaf) and no other
    /// tree, so 400 long root names leave the staged demand at two nodes.
    fn eviction_demand(&self) -> u64 {
        2
    }
}

struct Extending;

struct FallbackState {
    file: u64,
    peer: Option<u64>,
    entries: usize,
}

const EXTENDING_BEFORE: usize = BS + 200;

fn extending_after() -> Vec<u8> {
    let mut after = vec![0x21; EXTENDING_BEFORE];
    after.resize(BS + 450, 0);
    after[BS + 150..BS + 450].fill(0x7a);
    after
}

impl Family for Extending {
    type State = FallbackState;

    fn name(&self) -> &'static str {
        "extending-write fallback"
    }

    fn format(&self, variant: Variant) -> Format {
        policy_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> FallbackState {
        let file = volume
            .create_file_in_root("grow", &[0x21; EXTENDING_BEFORE], ts(2))
            .unwrap();
        volume
            .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, ts(3))
            .unwrap();
        FallbackState {
            file,
            peer: None,
            entries: 1,
        }
    }

    fn captured(&self, state: &FallbackState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, vec![0x21; EXTENDING_BEFORE])]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &FallbackState,
    ) -> Result<(), CoreError> {
        volume.write_file_at(state.file, (BS + 150) as u64, &[0x7a; 300], ts(20))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &FallbackState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(
            volume.lookup_root("grow").unwrap(),
            Some(state.file),
            "{context}"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.entries,
            "{context}"
        );
        flagged(volume, state.file, context);
        let expected = if delta == 1 {
            extending_after()
        } else {
            vec![0x21; EXTENDING_BEFORE]
        };
        assert_eq!(volume.read_file(state.file).unwrap(), expected, "{context}");
    }

    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &FallbackState,
        _variant: Variant,
    ) {
        let stats = volume.last_commit_stats().unwrap();
        assert_eq!(stats.data_blocks_overwritten_in_place, 0, "extending write");
    }
}

struct SharedBlock;

impl Family for SharedBlock {
    type State = FallbackState;

    fn name(&self) -> &'static str {
        "shared-block fallback"
    }

    fn format(&self, variant: Variant) -> Format {
        policy_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> FallbackState {
        let file = volume
            .create_file_in_root("origin", &[0x31; 2 * BS], ts(2))
            .unwrap();
        volume
            .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, ts(3))
            .unwrap();
        let peer = volume.clone_file(file, OBJECT_ROOT, "peer", ts(4)).unwrap();
        FallbackState {
            file,
            peer: Some(peer),
            entries: 2,
        }
    }

    fn captured(&self, state: &FallbackState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, vec![0x31; 2 * BS])]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &FallbackState,
    ) -> Result<(), CoreError> {
        volume.write_file_at(state.file, 50, &[0x4c; 100], ts(20))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &FallbackState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let peer = state.peer.unwrap();
        assert_eq!(
            volume.lookup_root("origin").unwrap(),
            Some(state.file),
            "{context}"
        );
        assert_eq!(volume.lookup_root("peer").unwrap(), Some(peer), "{context}");
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.entries,
            "{context}"
        );
        flagged(volume, state.file, context);
        let mut expected = vec![0x31; 2 * BS];
        if delta == 1 {
            expected[50..150].fill(0x4c);
        }
        assert_eq!(volume.read_file(state.file).unwrap(), expected, "{context}");
        assert_eq!(
            volume.read_file(peer).unwrap(),
            vec![0x31; 2 * BS],
            "{context}: peer"
        );
    }

    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &FallbackState,
        _variant: Variant,
    ) {
        let stats = volume.last_commit_stats().unwrap();
        assert_eq!(stats.data_blocks_overwritten_in_place, 0, "shared block");
    }
}

crate::profile_tests!(in_place_cuts_and_faults, |pages| matrix::plain(
    &InPlace, pages, 12
));
crate::profile_tests!(in_place_retained, |pages| matrix::retained(
    &InPlace, pages, 12
));
crate::profile_tests!(in_place_eviction, |pages| matrix::eviction(
    &InPlace,
    pages,
    Some(12)
));
crate::profile_tests!(in_place_ambiguous, |pages| matrix::ambiguous(
    &InPlace,
    pages,
    Variant::Plain
));
crate::profile_tests!(extending_fallback, |pages| matrix::plain(
    &Extending, pages, 12
));
crate::profile_tests!(shared_block_fallback, |pages| matrix::plain(
    &SharedBlock,
    pages,
    12
));
