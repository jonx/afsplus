//! One call resolves a whole AmigaDOS path. Every case resolves the same
//! path twice: once through `afsplus_aros_locate_path` and once through the
//! component loop the packet layer runs against a library without the
//! `PATHS` group, which is reproduced here from `resolve_path_lock` of
//! `native/aros/afsplus_packet.c`. The two must agree on the object, on what
//! `examine` reports, on the parent the lock names, and on the error.

mod common;

use afsplus_aros_ffi::*;
use common::{formatted, mount};

const SHARED: u32 = AFSPLUS_AROS_LOCK_SHARED;
const EXCLUSIVE: u32 = AFSPLUS_AROS_LOCK_EXCLUSIVE;
const ERROR_OBJECT_NOT_FOUND: i32 = 205;
const ERROR_OBJECT_WRONG_TYPE: i32 = 212;
const ERROR_IS_SOFT_LINK: i32 = 233;

/// What a resolved lock says about itself: the object, the name and type
/// `examine` reports, and the object its parent lock names (`None` at the
/// root). Two locks on the same object with the same history compare equal.
#[derive(Debug, PartialEq, Eq)]
struct Facts {
    object: u64,
    name: String,
    entry_type: i32,
    parent: Option<u64>,
}

fn examine(filesystem: *mut AfsplusAros, lock: u64) -> (AfsplusArosFileInfo, String) {
    let mut info = AfsplusArosFileInfo::default();
    let mut name = [0u8; 108];
    assert_eq!(
        afsplus_aros_examine_lock(filesystem, lock, &mut info, name.as_mut_ptr(), 108),
        0
    );
    let text = String::from_utf8(name[..info.name_length as usize].to_vec()).unwrap();
    (info, text)
}

fn facts(filesystem: *mut AfsplusAros, lock: u64) -> Facts {
    let (info, name) = examine(filesystem, lock);
    let mut parent = 0;
    assert_eq!(
        afsplus_aros_parent_lock_with_access(filesystem, lock, SHARED, &mut parent),
        0
    );
    let above = (parent != 0).then(|| {
        let (info, _) = examine(filesystem, parent);
        assert_eq!(afsplus_aros_free_lock(filesystem, parent), 0);
        info.object_id
    });
    Facts {
        object: info.object_id,
        name,
        entry_type: info.entry_type,
        parent: above,
    }
}

fn locate(filesystem: *mut AfsplusAros, base: u64, name: &[u8], access: u32) -> Result<u64, i32> {
    let mut lock = 0;
    let status = afsplus_aros_locate(
        filesystem,
        base,
        name.as_ptr(),
        name.len() as u32,
        access,
        &mut lock,
    );
    if status == 0 {
        Ok(lock)
    } else {
        Err(status)
    }
}

fn locate_path(
    filesystem: *mut AfsplusAros,
    base: u64,
    path: &[u8],
    access: u32,
) -> Result<u64, i32> {
    let mut lock = 0;
    let status = afsplus_aros_locate_path(
        filesystem,
        base,
        path.as_ptr(),
        path.len() as u32,
        access,
        &mut lock,
    );
    if status == 0 {
        Ok(lock)
    } else {
        Err(status)
    }
}

/// The next component as `next_path_operation` cuts it: up to the next `/`,
/// which is consumed with it, and nothing once the path is spent.
fn next_component<'a>(path: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    if *at >= path.len() {
        return None;
    }
    let start = *at;
    while *at < path.len() && path[*at] != b'/' {
        *at += 1;
    }
    let end = *at;
    if *at < path.len() {
        *at += 1;
    }
    Some(&path[start..end])
}

/// `resolve_path_lock` as it stands: one locate or parent operation per
/// component, the previous lock freed once the next one is made.
fn component_loop(
    filesystem: *mut AfsplusAros,
    base: u64,
    path: &[u8],
    access: u32,
) -> Result<u64, i32> {
    let start = path
        .iter()
        .rposition(|byte| *byte == b':')
        .map_or(0, |colon| colon + 1);
    let mut current = if start == 0 { base } else { 0 };
    let mut owned = false;
    let mut at = start;
    let free_temporary = |lock: u64, owned: bool| {
        if owned && lock != 0 {
            assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
        }
    };
    let Some(mut component) = next_component(path, &mut at) else {
        return locate(filesystem, current, &[], access);
    };
    loop {
        let last = at >= path.len();
        let step_access = if last { access } else { SHARED };
        let step = if component.is_empty() {
            let mut next = 0;
            let mut status = 0;
            if current != 0 {
                status = afsplus_aros_parent_lock_with_access(
                    filesystem,
                    current,
                    step_access,
                    &mut next,
                );
            }
            if status == 0 && last && next == 0 {
                match locate(filesystem, 0, &[], step_access) {
                    Ok(root) => next = root,
                    Err(error) => status = error,
                }
            }
            if status == 0 {
                Ok(next)
            } else {
                Err(status)
            }
        } else {
            locate(filesystem, current, component, step_access)
        };
        let next = match step {
            Ok(next) => next,
            Err(error) => {
                free_temporary(current, owned);
                return Err(error);
            }
        };
        free_temporary(current, owned);
        current = next;
        owned = current != 0;
        match next_component(path, &mut at) {
            Some(rest) => component = rest,
            None => return Ok(current),
        }
    }
}

/// Resolves `path` both ways, one at a time so that an exclusive request
/// does not collide with itself, and returns what both agreed on.
fn agree(
    filesystem: *mut AfsplusAros,
    base: u64,
    path: &str,
    access: u32,
) -> Result<Facts, i32> {
    let by_loop = component_loop(filesystem, base, path.as_bytes(), access).map(|lock| {
        let facts = facts(filesystem, lock);
        assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
        facts
    });
    let by_call = locate_path(filesystem, base, path.as_bytes(), access).map(|lock| {
        let facts = facts(filesystem, lock);
        assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
        facts
    });
    assert_eq!(by_call, by_loop, "path {path:?} with access {access}");
    by_call
}

fn make_directory(filesystem: *mut AfsplusAros, base: u64, name: &[u8]) -> u64 {
    let mut lock = 0;
    assert_eq!(
        afsplus_aros_create_directory(
            filesystem,
            base,
            name.as_ptr(),
            name.len() as u32,
            1,
            0,
            &mut lock
        ),
        0
    );
    lock
}

/// `one/two/three`, `one/x`, `other/x`, a file and a soft link to `one`.
fn tree(filesystem: *mut AfsplusAros) {
    let one = make_directory(filesystem, 0, b"one");
    let two = make_directory(filesystem, one, b"two");
    let three = make_directory(filesystem, two, b"three");
    let under_one = make_directory(filesystem, one, b"x");
    assert_eq!(afsplus_aros_free_lock(filesystem, three), 0);
    assert_eq!(afsplus_aros_free_lock(filesystem, under_one), 0);
    assert_eq!(afsplus_aros_free_lock(filesystem, two), 0);
    assert_eq!(afsplus_aros_free_lock(filesystem, one), 0);
    let other = make_directory(filesystem, 0, b"other");
    let under_other = make_directory(filesystem, other, b"x");
    assert_eq!(afsplus_aros_free_lock(filesystem, under_other), 0);
    assert_eq!(afsplus_aros_free_lock(filesystem, other), 0);
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            b"file".as_ptr(),
            4,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            1,
            0,
            &mut file
        ),
        0
    );
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
    assert_eq!(
        afsplus_aros_make_soft_link(
            filesystem,
            0,
            b"link".as_ptr(),
            4,
            b"one".as_ptr(),
            3,
            1,
            0
        ),
        0
    );
}

#[test]
fn one_call_resolves_the_path_the_component_loop_resolves() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    tree(filesystem);

    let root = agree(filesystem, 0, "", SHARED).unwrap();
    let deep = agree(filesystem, 0, "one/two/three", SHARED).unwrap();
    assert_eq!(deep.name, "three");
    let two = agree(filesystem, 0, "one/two", SHARED).unwrap();
    assert_eq!(deep.parent, Some(two.object));

    // A volume prefix and a leading colon both start at the root, whatever
    // the base lock is.
    let base = component_loop(filesystem, 0, b"one/two", SHARED).unwrap();
    assert_eq!(agree(filesystem, base, ":one/two", SHARED).unwrap(), two);
    assert_eq!(agree(filesystem, base, "AFS+:one/two", SHARED).unwrap(), two);
    assert_eq!(agree(filesystem, base, "AFS+:", SHARED).unwrap(), root);

    // An empty component is the parent operation, and a trailing separator
    // is inert.
    let one_x = agree(filesystem, 0, "one/x", SHARED).unwrap();
    assert_eq!(agree(filesystem, 0, "one/two//x", SHARED).unwrap(), one_x);
    assert_eq!(agree(filesystem, base, "/x", SHARED).unwrap(), one_x);
    assert_eq!(agree(filesystem, 0, "one/two/", SHARED).unwrap(), two);
    // AmigaDOS has no `..`: it is an ordinary name, and no object carries it.
    assert_eq!(
        agree(filesystem, 0, "one/two/three/..", SHARED),
        Err(ERROR_OBJECT_NOT_FOUND)
    );

    // The parent of a subdirectory, and at the root the parent of the root.
    let one = agree(filesystem, 0, "one", SHARED).unwrap();
    assert_eq!(agree(filesystem, base, "/", SHARED).unwrap(), one);
    let root_lock = component_loop(filesystem, 0, b"", SHARED).unwrap();
    assert_eq!(agree(filesystem, root_lock, "/", SHARED).unwrap(), root);
    assert_eq!(agree(filesystem, 0, "/", SHARED).unwrap(), root);
    assert_eq!(agree(filesystem, root_lock, "//", SHARED).unwrap(), root);
    assert_eq!(afsplus_aros_free_lock(filesystem, root_lock), 0);
    assert_eq!(afsplus_aros_free_lock(filesystem, base), 0);

    // The last component takes the access asked for; the ones before it are
    // passed through.
    let exclusive = agree(filesystem, 0, "one/two", EXCLUSIVE).unwrap();
    assert_eq!(exclusive, two);

    // The error is the error of the component that failed.
    assert_eq!(
        agree(filesystem, 0, "nowhere/three", SHARED),
        Err(ERROR_OBJECT_NOT_FOUND)
    );
    assert_eq!(
        agree(filesystem, 0, "one/nowhere/three", SHARED),
        Err(ERROR_OBJECT_NOT_FOUND)
    );
    assert_eq!(
        agree(filesystem, 0, "link/two", SHARED),
        Err(ERROR_IS_SOFT_LINK)
    );
    assert_eq!(agree(filesystem, 0, "link", SHARED), Err(ERROR_IS_SOFT_LINK));

    // A file is not a directory to walk through.
    let file = component_loop(filesystem, 0, b"file", SHARED).unwrap();
    assert_eq!(
        agree(filesystem, file, "one", SHARED),
        Err(ERROR_OBJECT_WRONG_TYPE)
    );
    assert_eq!(afsplus_aros_free_lock(filesystem, file), 0);

    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

/// The control: a name that exists under another parent must not be found
/// under the one the path names, and the same name under two parents must
/// resolve to the object of the parent that was walked.
#[test]
fn the_path_decides_which_object_the_lock_names() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    tree(filesystem);

    let one_x = locate_path(filesystem, 0, b"one/x", SHARED).unwrap();
    let one_x_facts = facts(filesystem, one_x);
    assert_eq!(afsplus_aros_free_lock(filesystem, one_x), 0);
    let other_x = locate_path(filesystem, 0, b"other/x", SHARED).unwrap();
    let other_x_facts = facts(filesystem, other_x);
    assert_eq!(afsplus_aros_free_lock(filesystem, other_x), 0);
    assert_eq!(one_x_facts.name, other_x_facts.name);
    assert_ne!(one_x_facts.object, other_x_facts.object);
    assert_ne!(one_x_facts.parent, other_x_facts.parent);

    // `three` is under `one/two` alone.
    assert_eq!(
        agree(filesystem, 0, "other/three", SHARED),
        Err(ERROR_OBJECT_NOT_FOUND)
    );
    assert_eq!(
        agree(filesystem, 0, "one/three", SHARED),
        Err(ERROR_OBJECT_NOT_FOUND)
    );

    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
