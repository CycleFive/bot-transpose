//! Opaque, platform-tagged, role-typed references.
//!
//! Three properties, each enforced by the type system rather than by care:
//! - **Role-typed**: a `RoomRef` cannot be passed where a `CommunityRef` is expected.
//! - **Platform-tagged**: Discord user `123` and Stoat user `123` are different values.
//! - **Opaque**: core code compares, hashes, stores and prints refs, and never parses
//!   or does arithmetic on them. Only an adapter knows what the raw text means.
//!
//! Storage is a column pair, `(platform, raw)`: the platform is a typed column
//! (a Postgres enum in `docs/schema.sql`) that can be extended or deprecated,
//! and the raw id is text. JSON mirrors it as `{"platform": "...", "id": "..."}`,
//! so the dashboard receives the same typed platform. A one-line key,
//! `<platform>:<raw>`, exists for logs, config files and map keys
//! (`discord:80351110224678912`, `matrix:@alice:example.org`).
//!
//! The kind is never stored: it comes from the column or field the pair is in.

use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The platforms this model has adapters for. Telegram appears only in the
/// capability tables, as a stress test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Platform {
    Discord,
    Matrix,
    Stoat,
    Telegram,
}

/// Where a platform is in its life. Extending the model is a new `Platform`
/// variant plus `ALTER TYPE platform ADD VALUE`. Deprecating one sets it to
/// `Deprecated`: rows that mention it stay readable, but no new ref can be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// Has an adapter; new refs welcome.
    Active,
    /// In the capability tables only, with no adapter. Telegram is here.
    Modelled,
    /// Had an adapter once; stored refs load, new ones are refused.
    Deprecated,
}

impl Lifecycle {
    pub const fn mints_refs(self) -> bool {
        matches!(self, Lifecycle::Active)
    }
}

impl Platform {
    pub const ALL: [Platform; 4] = [
        Platform::Discord,
        Platform::Matrix,
        Platform::Stoat,
        Platform::Telegram,
    ];

    /// The value in the `platform` column, and the prefix of a key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Platform::Discord => "discord",
            Platform::Matrix => "matrix",
            Platform::Stoat => "stoat",
            Platform::Telegram => "telegram",
        }
    }

    pub const fn lifecycle(self) -> Lifecycle {
        match self {
            Platform::Discord | Platform::Matrix | Platform::Stoat => Lifecycle::Active,
            Platform::Telegram => Lifecycle::Modelled,
        }
    }
}

impl FromStr for Platform {
    type Err = RefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Platform::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| RefError::UnknownPlatform(s.to_string()))
    }
}

/// What a reference points at. Sealed: the set of kinds is part of the model.
pub trait Kind: sealed::Sealed + 'static {
    const NAME: &'static str;
}

mod sealed {
    pub trait Sealed {}
}

macro_rules! kinds {
    ($($(#[$doc:meta])* $kind:ident => $alias:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $kind {}
        impl sealed::Sealed for $kind {}
        impl Kind for $kind {
            const NAME: &'static str = stringify!($kind);
        }
        pub type $alias = Ref<$kind>;
    )*};
}

kinds! {
    /// A person or bot account.
    User => UserRef,
    /// A Discord guild, a Stoat server, a Matrix space. Optional in a `Place`.
    Community => CommunityRef,
    /// Where messages (and maybe voice) happen: a channel, a Matrix room, a Telegram chat.
    Room => RoomRef,
    /// A thread under a room: a Discord thread, a Matrix thread root, a Telegram forum topic.
    Thread => ThreadRef,
    /// One message (a Matrix event id).
    Message => MessageRef,
    /// A role. Matrix has none: it uses power levels instead (see `authority`).
    Role => RoleRef,
}

/// The raw identifier as the platform spells it. Opaque to core.
///
/// One representation for every platform: Discord snowflakes, Telegram's negative
/// chat ids, Matrix ids with a server part and Stoat ULIDs are all text here.
/// Cloning is a reference-count bump; refs are cheap to clone but not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawId(Arc<str>);

impl RawId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A role-typed, platform-tagged, opaque reference.
pub struct Ref<K: Kind> {
    platform: Platform,
    raw: RawId,
    kind: PhantomData<K>,
}

impl<K: Kind> Ref<K> {
    /// A fresh ref, made by an adapter from a platform id. Refused for platforms
    /// that are not `Active`, so a deprecated platform gains no new rows.
    pub fn new(platform: Platform, raw: impl AsRef<str>) -> Result<Self, RefError> {
        if !platform.lifecycle().mints_refs() {
            return Err(RefError::NotMinting {
                platform,
                lifecycle: platform.lifecycle(),
            });
        }
        Self::from_stored(platform, raw)
    }

    /// A ref read back from storage or JSON. Any known platform loads, including
    /// a deprecated one. Empty raw ids and control characters are refused, so the
    /// pair and the key always round-trip.
    pub fn from_stored(platform: Platform, raw: impl AsRef<str>) -> Result<Self, RefError> {
        let raw = raw.as_ref();
        if raw.is_empty() || raw.chars().any(char::is_control) {
            return Err(RefError::BadRaw {
                kind: K::NAME,
                raw: raw.escape_debug().to_string(),
            });
        }
        Ok(Ref {
            platform,
            raw: RawId(Arc::from(raw)),
            kind: PhantomData,
        })
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// The raw text, for adapters converting back to a platform type.
    pub fn raw(&self) -> &RawId {
        &self.raw
    }

    /// The raw text, after checking the ref belongs to `platform`. This is the
    /// only way back out, so a Matrix room can never reach a Discord API call.
    pub fn raw_for(&self, platform: Platform) -> Result<&str, RefError> {
        if self.platform == platform {
            Ok(self.raw.as_str())
        } else {
            Err(RefError::WrongPlatform {
                kind: K::NAME,
                expected: platform,
                found: self.platform,
            })
        }
    }

    /// The column pair, for binding into a query.
    pub fn parts(&self) -> (Platform, &str) {
        (self.platform, self.raw.as_str())
    }

    /// The one-line key, `<platform>:<raw>`, for logs and config files.
    pub fn key(&self) -> String {
        format!("{}:{}", self.platform.as_str(), self.raw.as_str())
    }
}

// Manual impls: derives would demand `K: Clone` etc. on the uninhabited marker.
impl<K: Kind> Clone for Ref<K> {
    fn clone(&self) -> Self {
        Ref {
            platform: self.platform,
            raw: self.raw.clone(),
            kind: PhantomData,
        }
    }
}
impl<K: Kind> PartialEq for Ref<K> {
    fn eq(&self, other: &Self) -> bool {
        self.platform == other.platform && self.raw == other.raw
    }
}
impl<K: Kind> Eq for Ref<K> {}
impl<K: Kind> PartialOrd for Ref<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<K: Kind> Ord for Ref<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.platform, &self.raw).cmp(&(other.platform, &other.raw))
    }
}
impl<K: Kind> std::hash::Hash for Ref<K> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.platform.hash(state);
        self.raw.hash(state);
    }
}
impl<K: Kind> fmt::Debug for Ref<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", K::NAME, self.key())
    }
}
impl<K: Kind> fmt::Display for Ref<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key())
    }
}

impl<K: Kind> FromStr for Ref<K> {
    type Err = RefError;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        let (platform, raw) = key.split_once(':').ok_or_else(|| RefError::BadKey {
            kind: K::NAME,
            key: key.to_string(),
        })?;
        Ref::from_stored(platform.parse()?, raw)
    }
}

/// The JSON shape of a ref: the column pair, with the platform typed.
#[derive(Serialize, Deserialize)]
struct RefRepr<'a> {
    platform: Platform,
    #[serde(borrow)]
    id: std::borrow::Cow<'a, str>,
}

impl<K: Kind> Serialize for Ref<K> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        RefRepr {
            platform: self.platform,
            id: std::borrow::Cow::Borrowed(self.raw.as_str()),
        }
        .serialize(s)
    }
}
impl<'de, K: Kind> Deserialize<'de> for Ref<K> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = RefRepr::deserialize(d)?;
        Ref::from_stored(r.platform, r.id).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefError {
    #[error("{kind} ref has an unusable raw id {raw:?}")]
    BadRaw { kind: &'static str, raw: String },
    #[error("{kind} key {key:?} is not <platform>:<raw>")]
    BadKey { kind: &'static str, key: String },
    #[error("unknown platform {0:?}")]
    UnknownPlatform(String),
    #[error("{platform:?} is {lifecycle:?} and takes no new refs")]
    NotMinting {
        platform: Platform,
        lifecycle: Lifecycle,
    },
    #[error("{kind} ref belongs to {found:?}, not {expected:?}")]
    WrongPlatform {
        kind: &'static str,
        expected: Platform,
        found: Platform,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_including_colons_in_the_raw_id() {
        for key in [
            "discord:80351110224678912",
            "matrix:@alice:example.org",
            "matrix:!Jh7zLqcaM2D0xWnb:example.org",
            "stoat:01HV3W4QY7M0S0C5ZRJ4XK2B7D",
            "telegram:-1001234567890",
        ] {
            let r: UserRef = key.parse().unwrap();
            assert_eq!(r.key(), key);
            let json = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_json::from_str::<UserRef>(&json).unwrap(), r);
        }
    }

    #[test]
    fn json_is_the_column_pair_with_a_typed_platform() {
        let r = UserRef::new(Platform::Stoat, "01HV3W4QY7M0S0C5ZRJ4XK2B7D").unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"platform":"stoat","id":"01HV3W4QY7M0S0C5ZRJ4XK2B7D"}"#);
        assert_eq!(serde_json::from_str::<UserRef>(&json).unwrap(), r);
        assert_eq!(r.parts(), (Platform::Stoat, "01HV3W4QY7M0S0C5ZRJ4XK2B7D"));
        let bad = r#"{"platform":"irc","id":"x"}"#;
        assert!(serde_json::from_str::<UserRef>(bad).is_err());
    }

    #[test]
    fn a_modelled_platform_mints_no_refs_but_stored_ones_load() {
        assert_eq!(
            UserRef::new(Platform::Telegram, "1").unwrap_err(),
            RefError::NotMinting {
                platform: Platform::Telegram,
                lifecycle: Lifecycle::Modelled
            }
        );
        let stored = UserRef::from_stored(Platform::Telegram, "1").unwrap();
        let json = serde_json::to_string(&stored).unwrap();
        assert_eq!(serde_json::from_str::<UserRef>(&json).unwrap(), stored);
    }

    #[test]
    fn every_platform_column_value_parses_back() {
        for p in Platform::ALL {
            assert_eq!(p.as_str().parse::<Platform>().unwrap(), p);
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(json, format!("\"{}\"", p.as_str()));
        }
    }

    #[test]
    fn same_raw_on_two_platforms_is_two_values() {
        let d = UserRef::new(Platform::Discord, "123").unwrap();
        let s = UserRef::new(Platform::Stoat, "123").unwrap();
        assert_ne!(d, s);
    }

    #[test]
    fn raw_for_refuses_the_wrong_platform() {
        let m = RoomRef::new(Platform::Matrix, "!r:example.org").unwrap();
        assert!(m.raw_for(Platform::Discord).is_err());
        assert_eq!(m.raw_for(Platform::Matrix).unwrap(), "!r:example.org");
    }

    #[test]
    fn bad_keys_and_raws_are_errors() {
        assert!("nope".parse::<UserRef>().is_err());
        assert!("irc:alice".parse::<UserRef>().is_err());
        assert!(UserRef::new(Platform::Discord, "").is_err());
        assert!(UserRef::new(Platform::Discord, "1\n2").is_err());
    }
}
