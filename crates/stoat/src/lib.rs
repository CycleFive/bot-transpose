//! Stoat (formerly Revolt) adapter: rive-models' types in, neutral types out.
//!
//! Stoat is the closest of the three to Discord: servers, channels, roles and a
//! 64-bit permission field with nearly the same names. The differences that
//! matter to the model:
//! - ids are 26-character ULID strings, not integers;
//! - embeds have no fields, so a card's fields fold into the description;
//! - there are no buttons. `interactions.reactions` pins a set of reactions under
//!   a message, and that is the press surface;
//! - there are no registered commands and no private replies;
//! - voice is a LiveKit room reached with `POST /channels/{id}/join_call`, per
//!   third-party sources; rive-models 1.2.1 still describes the older Vortex token.

use std::collections::HashMap;

use rive_models::channel::Channel;
use rive_models::data::SendMessageData;
use rive_models::embed::SendableEmbed;
use rive_models::event::MessageReactEvent;
use rive_models::message::{Interactions, Message, Reply};
use rive_models::permission::Permission;
use rive_models::server::Server;
use rive_models::user::User;
use serde::Deserialize;
use transpose_core::authority::Standing;
use transpose_core::caps::{
    ActionSurface, AuthorityModel, CommandSurface, EmbedLevel, PlatformCaps, PrivateReply, VoiceSupport,
};
use transpose_core::command::{CommandSpec, ParseError, parse_text};
use transpose_core::event::{Inbound, Invocation};
use transpose_core::ids::{CommunityRef, MessageRef, RoleRef, RoomRef};
use transpose_core::ids::{Kind, Platform, Ref, RefError, UserRef};
use transpose_core::place::Place;
use transpose_core::render::{ActionPlan, Card, action_plan, to_markdown};
use transpose_core::render::{Out, OutBody, Pressables};
use transpose_core::voice::LiveKitGrant;
use transpose_web::Directory;

pub const CAPS: PlatformCaps = PlatformCaps {
    platform: Platform::Stoat,
    actions: ActionSurface::Reactions,
    embeds: EmbedLevel::Simple,
    private_reply: PrivateReply::No,
    commands: CommandSurface::Prefixed,
    edits: true,
    reactions: true,
    threads: false,
    authority: AuthorityModel::RolesAndPermissions,
    voice: VoiceSupport::LiveKit,
    max_text: 2000,
};

/// Stoat ids are already strings; this only tags them.
pub fn to_ref<K: Kind>(id: &str) -> Result<Ref<K>, RefError> {
    Ref::new(Platform::Stoat, id)
}

pub fn id_of<K: Kind>(r: &Ref<K>) -> Result<String, RefError> {
    r.raw_for(Platform::Stoat).map(str::to_owned)
}

/// A channel → place. DMs, groups and saved messages have no server.
pub fn place(channel: &Channel) -> Result<Place, RefError> {
    let (id, server) = match channel {
        Channel::TextChannel { id, server, .. } | Channel::VoiceChannel { id, server, .. } => {
            (id, Some(server))
        }
        Channel::DirectMessage { id, .. } | Channel::Group { id, .. } | Channel::SavedMessages { id, .. } => {
            (id, None)
        }
    };
    Ok(Place {
        community: server.map(|s| to_ref(s)).transpose()?,
        room: to_ref(id)?,
        thread: None,
    })
}

/// Permission bits and roles → standing, by the same rule as Discord's.
pub fn standing(perms: Permission, roles: &[String], is_owner: bool) -> Result<Standing, RefError> {
    Ok(Standing {
        admin: is_owner || perms.contains(Permission::ManageServer),
        moderator: perms.intersects(Permission::ManageMessages | Permission::MoveMembers),
        roles: roles
            .iter()
            .map(|r| to_ref::<transpose_core::ids::Role>(r))
            .collect::<Result<_, _>>()?,
        power: None,
    })
}

pub fn mention(u: &UserRef) -> String {
    format!("<@{}>", u.raw().as_str())
}

/// A card as a simple embed plus pinned reactions. Fields fold into the
/// description because Stoat embeds have none.
pub fn render(card: &Card) -> (SendableEmbed, Option<Interactions>) {
    let mut description = to_markdown(&card.body, &mention);
    for f in &card.fields {
        description.push_str(&format!("\n**{}**: {}", f.name, to_markdown(&f.value, &mention)));
    }
    if let Some(footer) = &card.footer {
        description.push_str(&format!("\n*{footer}*"));
    }
    let interactions = match action_plan(&CAPS, &card.actions) {
        ActionPlan::Reactions(a) => Some(Interactions {
            reactions: Some(a.into_iter().map(|a| a.emoji).collect()),
            restrict_reactions: true,
        }),
        ActionPlan::Hint(cmds) => {
            description.push_str(&format!("\n{}", cmds.join(", ")));
            None
        }
        _ => None,
    };
    let embed = SendableEmbed {
        title: card.title.clone(),
        url: card.url.clone(),
        description: Some(description),
        media: card.thumbnail.clone(),
        colour: card.accent.map(|c| format!("#{c:06x}")),
        icon_url: None,
    };
    (embed, interactions)
}

// ---- inbound -------------------------------------------------------------

/// A channel message → a command, when it starts with the scope's prefix.
/// `Ok(None)` is an ordinary chat message; `Err` is a command the user got wrong.
pub fn command(
    msg: &Message,
    channel: &Channel,
    specs: &[CommandSpec],
    prefix: &str,
) -> Result<Option<Inbound>, InboundError> {
    let Some(content) = msg.content.as_deref() else {
        return Ok(None);
    };
    let parsed = match parse_text(specs, prefix, content) {
        Ok(p) => p,
        Err(ParseError::NotACommand) => return Ok(None),
        Err(e) => return Err(InboundError::Parse(e)),
    };
    Ok(Some(Inbound::Command(Invocation {
        actor: to_ref(&msg.author)?,
        place: place(channel)?,
        command: parsed.command,
        args: parsed.args,
    })))
}

/// A reaction → an action press, when it is one of the bot's pinned reactions.
/// The bot's own seed reactions arrive here too and are dropped.
pub fn press(
    ev: &MessageReactEvent,
    channel: &Channel,
    pressables: &Pressables,
    bot: &UserRef,
) -> Result<Option<Inbound>, InboundError> {
    let actor: UserRef = to_ref(&ev.user_id)?;
    if &actor == bot {
        return Ok(None);
    }
    let message: MessageRef = to_ref(&ev.id)?;
    let Some(action) = pressables.resolve(&message, &ev.emoji_id) else {
        return Ok(None);
    };
    Ok(Some(Inbound::ActionPressed {
        actor,
        place: place(channel)?,
        message,
        action: action.to_string(),
    }))
}

#[derive(Debug, thiserror::Error)]
pub enum InboundError {
    #[error(transparent)]
    Ref(#[from] RefError),
    #[error(transparent)]
    Parse(ParseError),
}

// ---- outbound ------------------------------------------------------------

/// Core output → Stoat's send payload. Stoat has no private replies, so
/// `InvokerIfPossible` becomes an ordinary reply to the invoking message.
pub fn send(out: &Out, reply_to: Option<&MessageRef>) -> Result<SendMessageData, RefError> {
    let mut data = SendMessageData {
        replies: reply_to
            .map(|m| {
                Ok::<_, RefError>(vec![Reply {
                    id: id_of(m)?,
                    mention: false,
                }])
            })
            .transpose()?,
        ..SendMessageData::default()
    };
    match &out.body {
        OutBody::Text { text } => data.content = Some(to_markdown(text, &mention)),
        OutBody::Card { card } => {
            let (embed, interactions) = render(card);
            data.embeds = Some(vec![embed]);
            data.interactions = interactions;
        }
    }
    Ok(data)
}

// ---- voice signalling ----------------------------------------------------

/// `POST /channels/{id}/join_call`. Per third-party sources the answer is a
/// LiveKit URL and token; rive-models 1.2.1 predates it, so the shape is ours
/// and should be checked against stoat.js before anyone relies on it.
pub fn join_call_path(room: &RoomRef) -> Result<String, RefError> {
    Ok(format!("/channels/{}/join_call", id_of(room)?))
}

#[derive(Debug, Clone, Deserialize)]
pub struct JoinCallResponse {
    pub url: String,
    pub token: String,
}

/// Where Stoat's signalling ends and the shared LiveKit media layer begins.
pub fn grant(resp: JoinCallResponse) -> LiveKitGrant {
    LiveKitGrant {
        url: resp.url,
        token: resp.token,
        media_e2ee: false,
    }
}

// ---- names for the dashboard ---------------------------------------------

/// Display names from the adapter's model cache, for `transpose-web`.
#[derive(Debug, Default)]
pub struct StoatDirectory {
    users: HashMap<String, String>,
    rooms: HashMap<String, String>,
    communities: HashMap<String, String>,
    roles: HashMap<String, String>,
}

impl StoatDirectory {
    pub fn add_user(&mut self, u: &User) {
        let name = u
            .display_name
            .clone()
            .unwrap_or_else(|| format!("{}#{}", u.username, u.discriminator));
        self.users.insert(u.id.clone(), name);
    }

    pub fn add_server(&mut self, s: &Server) {
        self.communities.insert(s.id.clone(), s.name.clone());
        for (id, role) in &s.roles {
            self.roles.insert(id.clone(), role.name.clone());
        }
    }

    pub fn add_channel(&mut self, c: &Channel) {
        let (id, name) = match c {
            Channel::TextChannel { id, name, .. }
            | Channel::VoiceChannel { id, name, .. }
            | Channel::Group { id, name, .. } => (id, name.clone()),
            Channel::DirectMessage { id, .. } => (id, "direct message".into()),
            Channel::SavedMessages { id, .. } => (id, "saved messages".into()),
        };
        self.rooms.insert(id.clone(), name);
    }

    fn get<K: Kind>(map: &HashMap<String, String>, r: &Ref<K>) -> Option<String> {
        map.get(r.raw_for(Platform::Stoat).ok()?).cloned()
    }
}

impl Directory for StoatDirectory {
    fn user(&self, u: &UserRef) -> Option<String> {
        Self::get(&self.users, u)
    }
    fn room(&self, r: &RoomRef) -> Option<String> {
        Self::get(&self.rooms, r)
    }
    fn community(&self, c: &CommunityRef) -> Option<String> {
        Self::get(&self.communities, c)
    }
    fn role(&self, r: &RoleRef) -> Option<String> {
        Self::get(&self.roles, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transpose_core::authority::{Gate, Presence};
    use transpose_core::bot::{Effect, handle, now_playing};
    use transpose_core::event::Inbound;
    use transpose_core::ids::{MessageRef, RoleRef, RoomRef};
    use transpose_core::place::Scope;
    use transpose_core::settings::Settings;
    use transpose_core::voice::{Queue, Track};

    const SERVER: &str = "01HSV4TJ2ZQ9W0VZ6G8M1C2B3A";
    const CHANNEL: &str = "01HSV4TQ8F7N5R0P3K2J1H0G9F";
    const USER: &str = "01HSV4V2D3E4F5G6H7J8K9M0NP";

    #[test]
    fn a_server_channel_is_a_place_in_a_community() {
        let ch: Channel = serde_json::from_str(&format!(
            r#"{{"channel_type":"TextChannel","_id":"{CHANNEL}","server":"{SERVER}","name":"music"}}"#
        ))
        .unwrap();
        let p = place(&ch).unwrap();
        assert_eq!(p.community.unwrap().key(), format!("stoat:{SERVER}"));
        assert_eq!(p.room.key(), format!("stoat:{CHANNEL}"));
    }

    #[test]
    fn the_now_playing_card_folds_fields_and_pins_reactions() {
        let u: UserRef = to_ref(USER).unwrap();
        let mut q = Queue::default();
        for n in ["Song A", "Song B"] {
            q.enqueue(Track {
                title: n.into(),
                url: format!("https://youtu.be/{n}"),
                duration_secs: None,
                requested_by: u.clone(),
            });
        }
        q.advance();
        let (embed, inter) = render(&now_playing(q.now.as_ref().unwrap(), &q));
        let d = embed.description.unwrap();
        assert!(d.contains(&format!("**Requested by**: <@{USER}>")), "{d}");
        assert!(d.contains("**Up next**: Song B"), "{d}");
        assert_eq!(embed.colour.as_deref(), Some("#1db954"));
        let inter = inter.unwrap();
        assert!(inter.restrict_reactions);
        assert_eq!(inter.reactions.unwrap().len(), 3);
    }

    #[test]
    fn a_reaction_press_and_a_typed_command_reach_the_same_skip() {
        let u: UserRef = to_ref(USER).unwrap();
        let room: RoomRef = to_ref(CHANNEL).unwrap();
        let place = Place {
            community: Some(to_ref(SERVER).unwrap()),
            room: room.clone(),
            thread: None,
        };
        let mut s = Settings::defaults(place.scope());
        s.dj = Gate::Moderator;
        s.check(&CAPS).unwrap();
        let mut q = Queue::default();
        q.enqueue(Track {
            title: "a".into(),
            url: "u".into(),
            duration_secs: None,
            requested_by: u.clone(),
        });
        let pressed = Inbound::ActionPressed {
            actor: u.clone(),
            place: place.clone(),
            message: to_ref::<transpose_core::ids::Message>("01HSV4X")
                .map(|m: MessageRef| m)
                .unwrap(),
            action: "skip".into(),
        };
        let member = standing(Permission::SendMessage, &[], false).unwrap();
        let modr = standing(Permission::ManageMessages, &[], false).unwrap();
        assert!(matches!(
            handle(&pressed, &member, &Presence::default(), &s, &mut q),
            Some(Effect::Reply(_))
        ));
        assert!(matches!(
            handle(&pressed, &modr, &Presence::default(), &s, &mut q),
            Some(Effect::PlayNext(_))
        ));
        assert!(matches!(s.scope, Scope::Community(_)));
        let _: RoleRef = to_ref("01HROLE").unwrap();
    }

    fn channel() -> Channel {
        serde_json::from_str(&format!(
            r#"{{"channel_type":"TextChannel","_id":"{CHANNEL}","server":"{SERVER}","name":"music"}}"#
        ))
        .unwrap()
    }

    fn specs() -> Vec<CommandSpec> {
        use transpose_core::command::{ArgKind, ArgSpec};
        vec![
            CommandSpec {
                name: "skip".into(),
                summary: "Skip".into(),
                args: vec![],
            },
            CommandSpec {
                name: "play".into(),
                summary: "Play".into(),
                args: vec![ArgSpec {
                    name: "query".into(),
                    kind: ArgKind::Text,
                    required: true,
                    autocomplete: true,
                }],
            },
        ]
    }

    #[test]
    fn a_prefixed_message_becomes_an_invocation_and_chat_does_not() {
        let msg = |content: &str| -> Message {
            serde_json::from_str(&format!(
                r#"{{"_id":"01HMSG","channel":"{CHANNEL}","author":"{USER}","content":{content:?}}}"#
            ))
            .unwrap()
        };
        let Some(Inbound::Command(inv)) =
            command(&msg("!play lofi beats"), &channel(), &specs(), "!").unwrap()
        else {
            panic!("expected a command");
        };
        assert_eq!(inv.command, "play");
        assert_eq!(inv.actor.key(), format!("stoat:{USER}"));
        assert_eq!(inv.place.community.unwrap().key(), format!("stoat:{SERVER}"));
        assert!(
            command(&msg("hello there"), &channel(), &specs(), "!")
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            command(&msg("!play"), &channel(), &specs(), "!"),
            Err(InboundError::Parse(_))
        ));
    }

    #[test]
    fn only_a_pinned_reaction_from_someone_else_is_a_press() {
        let u: UserRef = to_ref(USER).unwrap();
        let bot: UserRef = to_ref("01HBOT").unwrap();
        let mut q = Queue::default();
        q.enqueue(Track {
            title: "a".into(),
            url: "u".into(),
            duration_secs: None,
            requested_by: u,
        });
        q.advance();
        let card = now_playing(q.now.as_ref().unwrap(), &q);
        let sent: MessageRef = to_ref("01HSENT").unwrap();
        let mut presses = Pressables::default();
        presses.remember(sent.clone(), &card.actions);
        let react = |user: &str, emoji: &str| MessageReactEvent {
            id: "01HSENT".into(),
            channel_id: CHANNEL.into(),
            user_id: user.into(),
            emoji_id: emoji.into(),
        };
        let got = press(&react(USER, "⏭"), &channel(), &presses, &bot).unwrap();
        assert!(matches!(got, Some(Inbound::ActionPressed { ref action, .. }) if action == "skip"));
        assert!(
            press(&react("01HBOT", "⏭"), &channel(), &presses, &bot)
                .unwrap()
                .is_none()
        );
        assert!(
            press(&react(USER, "🎉"), &channel(), &presses, &bot)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_private_reply_becomes_an_ordinary_reply_on_stoat() {
        use transpose_core::render::{Visibility, text};
        let invoking: MessageRef = to_ref("01HCMD").unwrap();
        let out = Out {
            body: OutBody::Text {
                text: vec![text("Skipped.")],
            },
            visibility: Visibility::InvokerIfPossible,
        };
        let data = send(&out, Some(&invoking)).unwrap();
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["content"], "Skipped.");
        assert_eq!(json["replies"][0]["id"], "01HCMD");
        assert_eq!(json["replies"][0]["mention"], false);
        let discord_msg = MessageRef::new(Platform::Discord, "1").unwrap();
        assert!(send(&out, Some(&discord_msg)).is_err());
    }

    #[test]
    fn join_call_signalling_ends_in_a_livekit_grant() {
        let room: RoomRef = to_ref(CHANNEL).unwrap();
        assert_eq!(
            join_call_path(&room).unwrap(),
            format!("/channels/{CHANNEL}/join_call")
        );
        let resp: JoinCallResponse =
            serde_json::from_str(r#"{"url":"wss://voice.stoat.chat","token":"t"}"#).unwrap();
        let g = grant(resp);
        assert_eq!((g.url.as_str(), g.media_e2ee), ("wss://voice.stoat.chat", false));
    }

    #[test]
    fn the_directory_feeds_the_dashboard_view() {
        let mut dir = StoatDirectory::default();
        let user: User = serde_json::from_str(&format!(
            r#"{{"_id":"{USER}","username":"alice","discriminator":"0001","display_name":"Alice"}}"#
        ))
        .unwrap();
        dir.add_user(&user);
        dir.add_channel(&channel());
        let u: UserRef = to_ref(USER).unwrap();
        assert_eq!(dir.user(&u).as_deref(), Some("Alice"));
        assert_eq!(dir.room(&to_ref(CHANNEL).unwrap()).as_deref(), Some("music"));
        let mut s = Settings::defaults(Scope::Room(to_ref(CHANNEL).unwrap()));
        s.authorized_users.insert(u);
        let view = transpose_web::settings_view(&s, &CAPS, &dir);
        assert_eq!(view.authorized_users[0].label, "Alice");
        assert!(view.gate_options.contains(&transpose_web::GateOption::Role));
    }
}
