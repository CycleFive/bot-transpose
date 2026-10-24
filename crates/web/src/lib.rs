//! What the dashboard (crack-web) sees.
//!
//! The dashboard speaks the neutral types too. It never holds a serenity or
//! ruma value: it gets `Settings` and `Queue` from core, asks a `Directory` for
//! display names, and sends back a `SettingsPatch` that core validates against
//! the platform's capabilities. The browser receives `PlatformCaps` with every
//! settings view, so the page can show a role picker on Discord and Stoat, a
//! power-level field on Matrix, and neither where a gate cannot exist.
//!
//! A dashboard login is a `UserRef` too: Discord OAuth2 today, then whatever
//! login each platform offers. That is outside this crate.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use transpose_core::authority::Gate;
use transpose_core::caps::{AuthorityModel, PlatformCaps};
use transpose_core::ids::{CommunityRef, Kind, Ref, RoleRef, RoomRef, UserRef};
use transpose_core::place::Scope;
use transpose_core::settings::{Settings, SettingsError};
use transpose_core::voice::{Queue, Track};

/// Display names for refs, from the adapter's cache. Missing names are normal
/// (a user left, the cache is cold), so every lookup is optional.
pub trait Directory {
    fn user(&self, u: &UserRef) -> Option<String>;
    fn room(&self, r: &RoomRef) -> Option<String>;
    fn community(&self, c: &CommunityRef) -> Option<String>;
    fn role(&self, r: &RoleRef) -> Option<String>;
}

/// A ref with the name to show. The label falls back to the ref's key, so the
/// page always has something to print.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(bound = "")]
pub struct Labeled<K: Kind> {
    #[serde(rename = "ref")]
    pub r: Ref<K>,
    pub label: String,
    pub known: bool,
}

fn label<K: Kind>(r: &Ref<K>, name: Option<String>) -> Labeled<K> {
    Labeled {
        known: name.is_some(),
        label: name.unwrap_or_else(|| r.key()),
        r: r.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopeView {
    Community {
        at: Labeled<transpose_core::ids::Community>,
    },
    Room {
        at: Labeled<transpose_core::ids::Room>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GateView {
    Anyone,
    Moderator,
    Admin,
    Role {
        role: Labeled<transpose_core::ids::Role>,
    },
    MinPower {
        level: i64,
    },
    Users {
        users: Vec<Labeled<transpose_core::ids::User>>,
    },
}

/// Which gate types this platform can save, so the page offers only those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateOption {
    Anyone,
    Moderator,
    Admin,
    Role,
    MinPower,
    Users,
}

pub fn gate_options(caps: &PlatformCaps) -> Vec<GateOption> {
    let mut o = vec![GateOption::Anyone, GateOption::Moderator, GateOption::Admin];
    match caps.authority {
        AuthorityModel::RolesAndPermissions => o.push(GateOption::Role),
        AuthorityModel::PowerLevels => o.push(GateOption::MinPower),
        AuthorityModel::AdminRights => {}
    }
    o.push(GateOption::Users);
    o
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsView {
    pub scope: ScopeView,
    pub caps: PlatformCaps,
    pub gate_options: Vec<GateOption>,
    pub prefix: String,
    pub volume: f32,
    pub autoplay: bool,
    pub private_replies: bool,
    pub dj: GateView,
    pub admin: GateView,
    pub authorized_users: Vec<Labeled<transpose_core::ids::User>>,
    pub ignored_rooms: Vec<Labeled<transpose_core::ids::Room>>,
}

fn gate_view(g: &Gate, dir: &dyn Directory) -> GateView {
    match g {
        Gate::Anyone => GateView::Anyone,
        Gate::Moderator => GateView::Moderator,
        Gate::Admin => GateView::Admin,
        Gate::Role(r) => GateView::Role {
            role: label(r, dir.role(r)),
        },
        Gate::MinPower(l) => GateView::MinPower { level: *l },
        Gate::Users(us) => GateView::Users {
            users: us.iter().map(|u| label(u, dir.user(u))).collect(),
        },
    }
}

pub fn settings_view(s: &Settings, caps: &PlatformCaps, dir: &dyn Directory) -> SettingsView {
    let scope = match &s.scope {
        Scope::Community(c) => ScopeView::Community {
            at: label(c, dir.community(c)),
        },
        Scope::Room(r) => ScopeView::Room {
            at: label(r, dir.room(r)),
        },
    };
    SettingsView {
        scope,
        caps: *caps,
        gate_options: gate_options(caps),
        prefix: s.prefix.clone(),
        volume: s.volume,
        autoplay: s.autoplay,
        private_replies: s.private_replies,
        dj: gate_view(&s.dj, dir),
        admin: gate_view(&s.admin, dir),
        authorized_users: s.authorized_users.iter().map(|u| label(u, dir.user(u))).collect(),
        ignored_rooms: s.ignored_rooms.iter().map(|r| label(r, dir.room(r))).collect(),
    }
}

/// A change from the page. Absent fields stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub prefix: Option<String>,
    pub volume: Option<f32>,
    pub autoplay: Option<bool>,
    pub private_replies: Option<bool>,
    pub dj: Option<Gate>,
    pub admin: Option<Gate>,
    pub authorized_users: Option<BTreeSet<UserRef>>,
    pub ignored_rooms: Option<BTreeSet<RoomRef>>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PatchError {
    #[error("volume must be between 0 and 2")]
    Volume,
    #[error("the prefix must be 1 to 5 characters with no spaces")]
    Prefix,
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

/// Apply a patch, then validate the whole result against the platform. The
/// stored settings change only if this returns Ok.
pub fn apply(current: &Settings, patch: SettingsPatch, caps: &PlatformCaps) -> Result<Settings, PatchError> {
    let mut s = current.clone();
    if let Some(p) = patch.prefix {
        if p.is_empty() || p.chars().count() > 5 || p.chars().any(char::is_whitespace) {
            return Err(PatchError::Prefix);
        }
        s.prefix = p;
    }
    if let Some(v) = patch.volume {
        if !(0.0..=2.0).contains(&v) {
            return Err(PatchError::Volume);
        }
        s.volume = v;
    }
    if let Some(a) = patch.autoplay {
        s.autoplay = a;
    }
    if let Some(p) = patch.private_replies {
        s.private_replies = p;
    }
    if let Some(g) = patch.dj {
        s.dj = g;
    }
    if let Some(g) = patch.admin {
        s.admin = g;
    }
    if let Some(u) = patch.authorized_users {
        s.authorized_users = u;
    }
    if let Some(r) = patch.ignored_rooms {
        s.ignored_rooms = r;
    }
    s.check(caps)?;
    Ok(s)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackView {
    pub title: String,
    pub url: String,
    pub duration_secs: Option<u32>,
    pub requested_by: Labeled<transpose_core::ids::User>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueueView {
    pub now: Option<TrackView>,
    pub next: Vec<TrackView>,
}

pub fn queue_view(q: &Queue, dir: &dyn Directory) -> QueueView {
    let track = |t: &Track| TrackView {
        title: t.title.clone(),
        url: t.url.clone(),
        duration_secs: t.duration_secs,
        requested_by: label(&t.requested_by, dir.user(&t.requested_by)),
    };
    QueueView {
        now: q.now.as_ref().map(track),
        next: q.next.iter().map(track).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transpose_core::authority::GateError;
    use transpose_core::caps::{ActionSurface, CommandSurface, EmbedLevel, PrivateReply, VoiceSupport};
    use transpose_core::ids::Platform;

    // The web crate must not depend on adapters, so the tests spell out the two
    // capability tables they need.
    fn caps(platform: Platform, authority: AuthorityModel) -> PlatformCaps {
        PlatformCaps {
            platform,
            actions: ActionSurface::Reactions,
            embeds: EmbedLevel::Simple,
            private_reply: PrivateReply::No,
            commands: CommandSurface::Prefixed,
            edits: true,
            reactions: true,
            threads: false,
            authority,
            voice: VoiceSupport::LiveKit,
            max_text: 2000,
        }
    }

    struct Names;
    impl Directory for Names {
        fn user(&self, u: &UserRef) -> Option<String> {
            (u.raw().as_str() == "01HALICE").then(|| "alice".into())
        }
        fn room(&self, _: &RoomRef) -> Option<String> {
            None
        }
        fn community(&self, _: &CommunityRef) -> Option<String> {
            Some("Lofi Lounge".into())
        }
        fn role(&self, _: &RoleRef) -> Option<String> {
            Some("DJ".into())
        }
    }

    fn stoat_settings() -> Settings {
        let mut s = Settings::defaults(Scope::Community(
            CommunityRef::new(Platform::Stoat, "01HSERVER").unwrap(),
        ));
        s.dj = Gate::Role(RoleRef::new(Platform::Stoat, "01HROLE").unwrap());
        s.authorized_users
            .insert(UserRef::new(Platform::Stoat, "01HALICE").unwrap());
        s.authorized_users
            .insert(UserRef::new(Platform::Stoat, "01HGONE").unwrap());
        s
    }

    #[test]
    fn the_settings_view_carries_names_caps_and_gate_options() {
        let stoat = caps(Platform::Stoat, AuthorityModel::RolesAndPermissions);
        let v = settings_view(&stoat_settings(), &stoat, &Names);
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["scope"]["kind"], "community");
        assert_eq!(json["scope"]["at"]["label"], "Lofi Lounge");
        assert_eq!(json["scope"]["at"]["ref"]["platform"], "stoat");
        assert_eq!(json["dj"]["type"], "role");
        assert_eq!(json["dj"]["role"]["label"], "DJ");
        assert_eq!(json["caps"]["authority"], "roles_and_permissions");
        assert!(v.gate_options.contains(&GateOption::Role));
        assert!(!v.gate_options.contains(&GateOption::MinPower));
        let users: Vec<_> = v
            .authorized_users
            .iter()
            .map(|u| (u.label.as_str(), u.known))
            .collect();
        assert_eq!(users, [("alice", true), ("stoat:01HGONE", false)]);
    }

    #[test]
    fn a_patch_from_the_page_is_validated_against_the_platform() {
        let matrix = caps(Platform::Matrix, AuthorityModel::PowerLevels);
        let room = Scope::Room(RoomRef::new(Platform::Matrix, "!r:example.org").unwrap());
        let current = Settings::defaults(room);
        let role_gate: SettingsPatch =
            serde_json::from_str(r#"{"dj":{"type":"role","value":{"platform":"matrix","id":"x"}}}"#).unwrap();
        assert_eq!(
            apply(&current, role_gate, &matrix),
            Err(PatchError::Settings(SettingsError::Gate(GateError::NoRoles(
                AuthorityModel::PowerLevels
            ))))
        );
        let power: SettingsPatch =
            serde_json::from_str(r#"{"dj":{"type":"min_power","value":50},"volume":0.8}"#).unwrap();
        let s = apply(&current, power, &matrix).unwrap();
        assert_eq!((s.dj, s.volume), (Gate::MinPower(50), 0.8));
    }

    #[test]
    fn a_user_from_another_platform_and_bad_values_are_refused() {
        let stoat = caps(Platform::Stoat, AuthorityModel::RolesAndPermissions);
        let foreign: SettingsPatch =
            serde_json::from_str(r#"{"authorized_users":[{"platform":"discord","id":"42"}]}"#).unwrap();
        assert_eq!(
            apply(&stoat_settings(), foreign, &stoat),
            Err(PatchError::Settings(SettingsError::ForeignRef))
        );
        let loud = SettingsPatch {
            volume: Some(9.0),
            ..SettingsPatch::default()
        };
        assert_eq!(apply(&stoat_settings(), loud, &stoat), Err(PatchError::Volume));
        assert!(serde_json::from_str::<SettingsPatch>(r#"{"guild_id":1}"#).is_err());
    }

    #[test]
    fn the_queue_view_names_requesters() {
        let alice = UserRef::new(Platform::Stoat, "01HALICE").unwrap();
        let mut q = Queue::default();
        q.enqueue(Track {
            title: "Song A".into(),
            url: "https://youtu.be/a".into(),
            duration_secs: Some(200),
            requested_by: alice,
        });
        q.advance();
        let v = queue_view(&q, &Names);
        assert_eq!(v.now.unwrap().requested_by.label, "alice");
        assert!(v.next.is_empty());
    }
}
