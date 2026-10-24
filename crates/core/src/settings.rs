//! Settings keyed by `Scope`, holding typed refs.
//!
//! cracktunes' `GuildSettings` has one `GuildId` field, the one cut the
//! topology map shows. It also stores Discord ids as bare `u64`
//! (`authorized_users`, `authorized_groups`, `ignored_channels`). Those are
//! Discord-shaped data the map cannot see, because a `u64` is not a Discord type.
//! Here every one of them is a typed ref.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::authority::{Gate, GateError};
use crate::caps::PlatformCaps;
use crate::ids::{RoomRef, UserRef};
use crate::place::Scope;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub scope: Scope,
    /// Text-command prefix, for platforms that parse text.
    pub prefix: String,
    pub volume: f32,
    pub autoplay: bool,
    /// Who may control playback (skip, pause, volume).
    pub dj: Gate,
    /// Who may change these settings.
    pub admin: Gate,
    pub authorized_users: BTreeSet<UserRef>,
    pub ignored_rooms: BTreeSet<RoomRef>,
    pub private_replies: bool,
}

impl Settings {
    pub fn defaults(scope: Scope) -> Self {
        Settings {
            scope,
            prefix: "!".into(),
            volume: 0.5,
            autoplay: true,
            dj: Gate::Anyone,
            admin: Gate::Admin,
            authorized_users: BTreeSet::new(),
            ignored_rooms: BTreeSet::new(),
            private_replies: false,
        }
    }

    /// Validate before saving: gates must be expressible on this platform, and
    /// every ref must belong to it.
    pub fn check(&self, caps: &PlatformCaps) -> Result<(), SettingsError> {
        self.dj.check(caps)?;
        self.admin.check(caps)?;
        let scope_platform = match &self.scope {
            Scope::Community(c) => c.platform(),
            Scope::Room(r) => r.platform(),
        };
        let foreign = scope_platform != caps.platform
            || self.authorized_users.iter().any(|u| u.platform() != caps.platform)
            || self.ignored_rooms.iter().any(|r| r.platform() != caps.platform);
        if foreign {
            return Err(SettingsError::ForeignRef);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    Gate(#[from] GateError),
    #[error("a setting names something from another platform")]
    ForeignRef,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CommunityRef, Platform};

    #[test]
    fn settings_round_trip_through_json_with_keys() {
        let scope = Scope::Community(CommunityRef::new(Platform::Stoat, "01HSERVER").unwrap());
        let mut s = Settings::defaults(scope);
        s.authorized_users.insert(UserRef::new(Platform::Stoat, "01HUSER").unwrap());
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"stoat:01HUSER\""), "{json}");
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
    }
}
