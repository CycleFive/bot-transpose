//! Who may do what. The adapter turns the platform's model (permission bits and
//! roles, power levels, admin rights) into a `Standing`; core policy decides
//! with plain values. Settings speak in `Gate`s, which are checked against the
//! platform's capabilities when they are saved, so a role gate can never be
//! configured on Matrix.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::caps::{AuthorityModel, PlatformCaps};
use crate::ids::{RoleRef, RoomRef, UserRef};

/// What the adapter knows about an actor at one place, in neutral terms.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Standing {
    /// Owner, Discord ADMINISTRATOR or MANAGE_GUILD, Stoat ManageServer, Matrix
    /// creator (room v12 "infinite" power) or enough power to change power levels.
    pub admin: bool,
    /// Can moderate here: manage messages or move members (Discord, Stoat), or a
    /// power level at or above the room's kick level (Matrix).
    pub moderator: bool,
    pub roles: BTreeSet<RoleRef>,
    /// Matrix only. `Level::Infinite` is a room-v12 creator.
    pub power: Option<Level>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Int(i64),
    Infinite,
}

/// A rule a setting uses to say who may do something.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Gate {
    Anyone,
    Moderator,
    Admin,
    /// Discord and Stoat: has this role.
    Role(RoleRef),
    /// Matrix: power level at least this.
    MinPower(i64),
    /// Anyone in this explicit list.
    Users(BTreeSet<UserRef>),
}

impl Gate {
    /// Refuse a gate the platform cannot express. Called when settings are saved.
    pub fn check(&self, caps: &PlatformCaps) -> Result<(), GateError> {
        match (self, caps.authority) {
            (Gate::Role(r), AuthorityModel::RolesAndPermissions) if r.platform() == caps.platform => Ok(()),
            (Gate::Role(_), _) => Err(GateError::NoRoles(caps.authority)),
            (Gate::MinPower(_), AuthorityModel::PowerLevels) => Ok(()),
            (Gate::MinPower(_), _) => Err(GateError::NoPowerLevels(caps.authority)),
            (Gate::Users(us), _) if us.iter().any(|u| u.platform() != caps.platform) => {
                Err(GateError::ForeignUser)
            }
            _ => Ok(()),
        }
    }

    pub fn admits(&self, actor: &UserRef, s: &Standing) -> bool {
        if s.admin {
            return true;
        }
        match self {
            Gate::Anyone => true,
            Gate::Moderator => s.moderator,
            Gate::Admin => false,
            Gate::Role(r) => s.roles.contains(r),
            Gate::MinPower(min) => s.power.is_some_and(|p| p >= Level::Int(*min)),
            Gate::Users(us) => us.contains(actor),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    #[error("this platform has no roles ({0:?})")]
    NoRoles(AuthorityModel),
    #[error("this platform has no power levels ({0:?})")]
    NoPowerLevels(AuthorityModel),
    #[error("a gate names a user from another platform")]
    ForeignUser,
}

/// Where the actor and the bot are in voice. Music bots usually require the
/// actor to be in the bot's voice room to control playback.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Presence {
    pub actor_voice: Option<RoomRef>,
    pub bot_voice: Option<RoomRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(Denial),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denial {
    NotInBotsVoiceRoom,
    GateNotMet,
}

/// Playback control: the gate decides, and outside the admin and moderator
/// classes the actor must also be listening in the bot's room.
pub fn may_control(actor: &UserRef, standing: &Standing, presence: &Presence, gate: &Gate) -> Decision {
    if !gate.admits(actor, standing) {
        return Decision::Deny(Denial::GateNotMet);
    }
    let staff = standing.admin || standing.moderator;
    if !staff && presence.bot_voice.is_some() && presence.actor_voice != presence.bot_voice {
        return Decision::Deny(Denial::NotInBotsVoiceRoom);
    }
    Decision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::Platform;

    fn user(p: Platform, raw: &str) -> UserRef {
        UserRef::new(p, raw).unwrap()
    }

    #[test]
    fn role_gates_only_exist_where_roles_do() {
        let role = RoleRef::new(Platform::Matrix, "x").unwrap();
        assert_eq!(
            Gate::Role(role).check(&PlatformCaps::TELEGRAM),
            Err(GateError::NoRoles(AuthorityModel::AdminRights))
        );
        assert!(Gate::MinPower(50).check(&PlatformCaps::TELEGRAM).is_err());
    }

    #[test]
    fn infinite_power_passes_any_threshold() {
        let s = Standing {
            power: Some(Level::Infinite),
            ..Standing::default()
        };
        let u = user(Platform::Matrix, "@c:example.org");
        assert!(Gate::MinPower(i64::MAX).admits(&u, &s));
        assert!(!Gate::MinPower(1).admits(&u, &Standing::default()));
    }

    #[test]
    fn listeners_must_share_the_bots_room_but_moderators_need_not() {
        let room = RoomRef::new(Platform::Stoat, "A").unwrap();
        let other = RoomRef::new(Platform::Stoat, "B").unwrap();
        let u = user(Platform::Stoat, "u");
        let away = Presence {
            actor_voice: Some(other),
            bot_voice: Some(room),
        };
        assert_eq!(
            may_control(&u, &Standing::default(), &away, &Gate::Anyone),
            Decision::Deny(Denial::NotInBotsVoiceRoom)
        );
        let modr = Standing {
            moderator: true,
            ..Standing::default()
        };
        assert_eq!(may_control(&u, &modr, &away, &Gate::Anyone), Decision::Allow);
    }
}
