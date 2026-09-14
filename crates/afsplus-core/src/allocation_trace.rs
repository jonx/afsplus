//! Optional host-qualification allocation origins, not current object ownership.
//! Default builds erase scopes; feature builds activate them explicitly at runtime.
use std::marker::PhantomData;

pub const COUNT: usize = 9;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Domain {
    Other,
    Fixture,
    Reporting,
    Allocator,
    Tree,
    Batch,
    Snapshot,
    Verifier,
    Oracle,
}
pub const NAMES: [&str; COUNT] = [
    "other",
    "fixture",
    "reporting",
    "allocator",
    "tree",
    "batch",
    "snapshot",
    "verifier",
    "oracle",
];

#[cfg(feature = "allocation-tracing")]
mod active {
    use super::Domain;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    pub static ENABLED: AtomicBool = AtomicBool::new(false);
    // Const, non-dropping TLS avoids lazy allocator-backed initialization.
    thread_local! { pub static CURRENT: Cell<Domain> = const { Cell::new(Domain::Other) }; }
    pub fn enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }
}

/// One-way activation for a host qualification process, not filesystem policy.
pub fn enable() {
    #[cfg(feature = "allocation-tracing")]
    active::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn current() -> Domain {
    #[cfg(feature = "allocation-tracing")]
    {
        active::CURRENT
            .try_with(|value| value.get())
            .unwrap_or(Domain::Other)
    }
    #[cfg(not(feature = "allocation-tracing"))]
    {
        Domain::Other
    }
}

/// The guard cannot move between threads; it restores nesting on unwinding.
pub struct Scope {
    #[cfg(feature = "allocation-tracing")]
    previous: Option<Domain>,
    marker: PhantomData<std::rc::Rc<()>>,
}

#[inline]
pub fn enter(domain: Domain) -> Scope {
    #[cfg(feature = "allocation-tracing")]
    let previous = if active::enabled() {
        active::CURRENT.try_with(|value| value.replace(domain)).ok()
    } else {
        None
    };
    let _ = domain;
    Scope {
        #[cfg(feature = "allocation-tracing")]
        previous,
        marker: PhantomData,
    }
}

#[cfg(feature = "allocation-tracing")]
impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            let _ = active::CURRENT.try_with(|value| value.set(previous));
        }
    }
}

#[inline]
pub fn within<T>(domain: Domain, work: impl FnOnce() -> T) -> T {
    let _scope = enter(domain);
    work()
}
