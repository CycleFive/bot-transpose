//! Inbound events, already translated by an adapter.

use crate::command::ArgValue;
use crate::ids::{MessageRef, RoomRef, UserRef};
use crate::place::Place;

#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub actor: UserRef,
    pub place: Place,
    pub command: String,
    pub args: Vec<(String, ArgValue)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    /// A registered command (Discord) or parsed text (Matrix, Stoat, Telegram).
    Command(Invocation),
    /// A button press (Discord, Telegram) or a reaction mapped back to its
    /// action by emoji (Matrix, Stoat). Core cannot tell which, by design.
    ActionPressed {
        actor: UserRef,
        place: Place,
        message: MessageRef,
        action: String,
    },
    VoiceChanged {
        user: UserRef,
        from: Option<RoomRef>,
        to: Option<RoomRef>,
    },
}
