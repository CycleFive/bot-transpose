//! Where something happens, and which settings apply there.
//!
//! Discord's guild-then-channel pair is one shape of a more general tree:
//! an optional community, a room, and an optional thread. On Matrix a room may sit
//! in no space, or in several, so "the community of this room" is not always
//! defined. The model admits that instead of inventing a guild.

use serde::{Deserialize, Serialize};

use crate::ids::{CommunityRef, RoomRef, ThreadRef};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Place {
    /// None for DMs and group chats, and for a Matrix room the bot has not been
    /// told belongs to a space.
    pub community: Option<CommunityRef>,
    pub room: RoomRef,
    pub thread: Option<ThreadRef>,
}

/// What a room can host. Discord voice channels also have a text chat, and a
/// Matrix room can hold both a timeline and a call, so these are flags, not a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomCaps {
    pub chat: bool,
    pub voice: bool,
}

/// The unit settings attach to. `GuildSettings { guild_id, .. }` becomes
/// `Settings { scope, .. }`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Scope {
    Community(CommunityRef),
    Room(RoomRef),
}

impl Place {
    /// Settings follow the community when there is one, else the room.
    pub fn scope(&self) -> Scope {
        match &self.community {
            Some(c) => Scope::Community(c.clone()),
            None => Scope::Room(self.room.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::Platform;

    #[test]
    fn scope_falls_back_to_the_room_without_a_community() {
        let room = RoomRef::new(Platform::Matrix, "!r:example.org").unwrap();
        let alone = Place {
            community: None,
            room: room.clone(),
            thread: None,
        };
        assert_eq!(alone.scope(), Scope::Room(room.clone()));
        let space = CommunityRef::new(Platform::Matrix, "!space:example.org").unwrap();
        let in_space = Place {
            community: Some(space.clone()),
            ..alone
        };
        assert_eq!(in_space.scope(), Scope::Community(space));
    }
}
