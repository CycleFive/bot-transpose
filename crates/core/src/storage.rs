//! Row shapes for the `(platform, raw)` column pair.
//!
//! Core stays free of any database crate. These are the plain structs a
//! repository layer binds and reads; with sqlx, `Platform` would derive
//! `sqlx::Type` against the `platform` enum in `docs/schema.sql`. Reading goes
//! through `Ref::from_stored`, so rows naming a deprecated platform still load.

use crate::ids::{Kind, Platform, Ref, RefError};
use crate::place::Scope;

/// One ref as two columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefColumns {
    pub platform: Platform,
    pub raw: String,
}

impl<K: Kind> From<&Ref<K>> for RefColumns {
    fn from(r: &Ref<K>) -> Self {
        let (platform, raw) = r.parts();
        RefColumns {
            platform,
            raw: raw.to_string(),
        }
    }
}

impl RefColumns {
    pub fn load<K: Kind>(self) -> Result<Ref<K>, RefError> {
        Ref::from_stored(self.platform, self.raw)
    }
}

/// The `scope_kind` column: which kind of ref the scope pair holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Community,
    Room,
}

impl ScopeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ScopeKind::Community => "community",
            ScopeKind::Room => "room",
        }
    }
}

/// A settings scope as three columns: `(scope_kind, scope_platform, scope_raw)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeColumns {
    pub kind: ScopeKind,
    pub at: RefColumns,
}

impl From<&Scope> for ScopeColumns {
    fn from(s: &Scope) -> Self {
        match s {
            Scope::Community(c) => ScopeColumns {
                kind: ScopeKind::Community,
                at: c.into(),
            },
            Scope::Room(r) => ScopeColumns {
                kind: ScopeKind::Room,
                at: r.into(),
            },
        }
    }
}

impl ScopeColumns {
    pub fn load(self) -> Result<Scope, RefError> {
        Ok(match self.kind {
            ScopeKind::Community => Scope::Community(self.at.load()?),
            ScopeKind::Room => Scope::Room(self.at.load()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CommunityRef, RoomRef, UserRef};

    #[test]
    fn refs_and_scopes_round_trip_through_columns() {
        let u = UserRef::new(Platform::Matrix, "@alice:example.org").unwrap();
        let cols = RefColumns::from(&u);
        assert_eq!(cols.platform, Platform::Matrix);
        assert_eq!(cols.clone().load::<crate::ids::User>().unwrap(), u);
        for scope in [
            Scope::Community(CommunityRef::new(Platform::Stoat, "01HSERVER").unwrap()),
            Scope::Room(RoomRef::new(Platform::Matrix, "!r:example.org").unwrap()),
        ] {
            assert_eq!(ScopeColumns::from(&scope).load().unwrap(), scope);
        }
    }

    #[test]
    fn a_row_with_an_empty_raw_id_is_an_error_not_a_ref() {
        let cols = RefColumns {
            platform: Platform::Discord,
            raw: String::new(),
        };
        assert!(cols.load::<crate::ids::User>().is_err());
    }
}
