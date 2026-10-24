//! What a platform can do, as data. Core code asks "can I show buttons here?",
//! never "am I on Discord?". Each adapter publishes one `PlatformCaps` constant.

use serde::{Deserialize, Serialize};

use crate::ids::Platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformCaps {
    pub platform: Platform,
    /// How the platform lets a user press an action on a message.
    pub actions: ActionSurface,
    pub embeds: EmbedLevel,
    /// Can a reply be shown to the invoker only?
    pub private_reply: PrivateReply,
    pub commands: CommandSurface,
    pub edits: bool,
    pub reactions: bool,
    pub threads: bool,
    /// Discord and Stoat have roles; Matrix has power levels; Telegram has admin rights.
    pub authority: AuthorityModel,
    pub voice: VoiceSupport,
    /// Longest plain message, in characters.
    pub max_text: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSurface {
    /// Real buttons with an id (Discord components, Telegram inline keyboards).
    Buttons,
    /// The bot seeds emoji reactions and treats a user's reaction as a press
    /// (Stoat `interactions.reactions`, Matrix `m.reaction`).
    Reactions,
    /// No way to press: the message shows the text command instead.
    TextHint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbedLevel {
    /// Title, description, fields, thumbnail, footer, colour (Discord).
    Rich,
    /// Title, description, one media item, colour; no fields (Stoat).
    Simple,
    /// None: the card becomes formatted text (Matrix HTML, Telegram HTML).
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateReply {
    /// Any reply to an interaction (Discord ephemeral).
    Full,
    /// A short toast only, in answer to a button press (Telegram `answerCallbackQuery`).
    ToastOnly,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSurface {
    /// Registered commands with typed options and autocomplete (Discord).
    Registered { autocomplete: bool },
    /// A registered menu of names, but arguments are free text (Telegram `setMyCommands`).
    Menu,
    /// Text with a prefix, parsed by the bot (Matrix, Stoat).
    Prefixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityModel {
    RolesAndPermissions,
    PowerLevels,
    AdminRights,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSupport {
    /// The platform's own voice transport (Discord: voice gateway + UDP + DAVE E2EE).
    Native,
    /// A LiveKit room, reached through platform signalling (Matrix MatrixRTC, Stoat join_call).
    LiveKit,
    /// Not with a bot account (Telegram needs a separate user session).
    Unsupported,
}

impl PlatformCaps {
    /// The Telegram column: modelled so the design is tested against a platform
    /// with buttons but no bot voice. No adapter crate exists for it.
    pub const TELEGRAM: PlatformCaps = PlatformCaps {
        platform: Platform::Telegram,
        actions: ActionSurface::Buttons,
        embeds: EmbedLevel::None,
        private_reply: PrivateReply::ToastOnly,
        commands: CommandSurface::Menu,
        edits: true,
        reactions: true,
        threads: true,
        authority: AuthorityModel::AdminRights,
        voice: VoiceSupport::Unsupported,
        max_text: 4096,
    };
}
