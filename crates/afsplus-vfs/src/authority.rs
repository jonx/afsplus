//! Shared private operation-admission machinery. Public roles wrap these types
//! separately, so a backup grant cannot be passed as restore authority.
use std::sync::{Arc, RwLock, RwLockReadGuard};

#[derive(Clone)]
pub(crate) struct Scope(Arc<()>);
pub(crate) struct GrantState {
    scope: Scope,
    pub(crate) active: RwLock<bool>,
}
#[derive(Clone)]
pub(crate) struct Grant(pub(crate) Arc<GrantState>);
#[derive(Clone)]
pub(crate) struct Issuer {
    scope: Scope,
}
pub(crate) struct Denied;

impl Scope {
    pub(crate) fn new() -> (Self, Issuer) {
        let scope = Self(Arc::new(()));
        let issuer = Issuer {
            scope: scope.clone(),
        };
        (scope, issuer)
    }
    fn owns(&self, grant: &Grant) -> bool {
        Arc::ptr_eq(&self.0, &grant.0.scope.0)
    }
    pub(crate) fn admit<'a>(&self, grant: &'a Grant) -> Result<RwLockReadGuard<'a, bool>, Denied> {
        if !self.owns(grant) {
            return Err(Denied);
        }
        let permit = grant.0.active.read().map_err(|_| Denied)?;
        if !*permit {
            return Err(Denied);
        }
        Ok(permit)
    }
}
impl Grant {
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Issuer {
    pub(crate) fn grant(&self) -> Grant {
        Grant(Arc::new(GrantState {
            scope: self.scope.clone(),
            active: RwLock::new(true),
        }))
    }
    pub(crate) fn revoke(&self, grant: &Grant) -> Result<(), Denied> {
        if !self.scope.owns(grant) {
            return Err(Denied);
        }
        let mut active = grant.0.active.write().map_err(|_| Denied)?;
        *active = false;
        Ok(())
    }
}
