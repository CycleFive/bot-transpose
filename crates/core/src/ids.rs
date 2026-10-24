//! Opaque, platform-tagged, role-typed references.
//!
//! Three properties, each enforced by the type system rather than by care:
//! - **Role-typed**: a `RoomRef` cannot be passed where a `CommunityRef` is expected.
//! - **Platform-tagged**: Discord user `123` and Stoat user `123` are different values.
//! - **Opaque**: core code compares, hashes, stores and prints refs, and never parses
//!   or does arithmetic on them. Only an adapter knows what the raw text means.
//!
//! Refs serialise as one stable string, `<platform>:<raw>`, for example
//! `discord:80351110224678912`, `matrix:@alice:example.org`, or
//! `stoat:01HV3W4QY7M0S0C5ZRJ4XK2B7D`. The kind is not in the key: it comes from
//! the column or field the key is stored in.

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

impl Platform {
    pub const fn key(self) -> &'static str {
        match self {
            Platform::Discord => "discord",
            Platform::Matrix => "matrix",
            Platform::Stoat => "stoat",
            Platform::Telegram => "telegram",
        }
    }

    fn from_key(s: &str) -> Option<Self> {
        Some(match s {
            "discord" => Platform::Discord,
            "matrix" => Platform::Matrix,
            "stoat" => Platform::Stoat,
            "telegram" => Platform::Telegram,
            _ => return None,
        })
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
    /// Adapters build refs; core code only receives them. Empty raw ids are refused,
    /// as is anything with control characters, so a key always round-trips.
    pub fn new(platform: Platform, raw: impl AsRef<str>) -> Result<Self, RefError> {
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

    /// The stable storage key, `<platform>:<raw>`.
    pub fn key(&self) -> String {
        format!("{}:{}", self.platform.key(), self.raw.as_str())
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
        let platform = Platform::from_key(platform).ok_or_else(|| RefError::BadKey {
            kind: K::NAME,
            key: key.to_string(),
        })?;
        Ref::new(platform, raw)
    }
}

impl<K: Kind> Serialize for Ref<K> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.key())
    }
}
impl<'de, K: Kind> Deserialize<'de> for Ref<K> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let key = String::deserialize(d)?;
        key.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefError {
    #[error("{kind} ref has an unusable raw id {raw:?}")]
    BadRaw { kind: &'static str, raw: String },
    #[error("{kind} key {key:?} is not <platform>:<raw>")]
    BadKey { kind: &'static str, key: String },
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
