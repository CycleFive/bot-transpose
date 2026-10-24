//! Matrix adapter: ruma's types in, neutral types out.
//!
//! Matrix differs from Discord in shape, not just spelling:
//! - A room may be in no space or in several, so `Place::community` is set only
//!   when the bot is told which space a room belongs to.
//! - There are no roles. Authority is a power level per room, and from room
//!   version 12 a room's creators have "infinite" power.
//! - There are no buttons or embeds. A card is HTML (`formatted_body`) with a
//!   plain `body`, and actions become reactions the bot seeds on its message.

use ruma::events::StateEventType;
use ruma::events::reaction::ReactionEventContent;
use ruma::events::relation::Annotation;
use ruma::events::room::message::{MessageType, RoomMessageEventContent};
use ruma::events::room::power_levels::{RoomPowerLevels, UserPowerLevel};
use ruma::{OwnedEventId, OwnedRoomId, OwnedUserId, UserId};
use serde::Deserialize;
use transpose_core::authority::{Level, Standing};
use transpose_core::caps::{
    ActionSurface, AuthorityModel, CommandSurface, EmbedLevel, PlatformCaps, PrivateReply, VoiceSupport,
};
use transpose_core::command::{CommandSpec, ParseError, parse_text};
use transpose_core::event::{Inbound, Invocation};
use transpose_core::ids::{CommunityRef, MessageRef, Platform, RefError, RoomRef, ThreadRef, UserRef};
use transpose_core::place::Place;
use transpose_core::render::Pressables;
use transpose_core::render::{ActionPlan, Card, Span, action_plan, to_html, to_plain};
use transpose_core::voice::LiveKitGrant;

pub const CAPS: PlatformCaps = PlatformCaps {
    platform: Platform::Matrix,
    actions: ActionSurface::Reactions,
    embeds: EmbedLevel::None,
    private_reply: PrivateReply::No,
    commands: CommandSurface::Prefixed,
    edits: true,
    reactions: true,
    threads: true,
    authority: AuthorityModel::PowerLevels,
    voice: VoiceSupport::LiveKit,
    max_text: 65_536,
};

fn r<K: transpose_core::ids::Kind>(s: &str) -> transpose_core::ids::Ref<K> {
    transpose_core::ids::Ref::new(Platform::Matrix, s).expect("a parsed Matrix id is a valid raw id")
}

pub fn user_ref(u: &UserId) -> UserRef {
    r(u.as_str())
}
pub fn room_ref(id: &OwnedRoomId) -> RoomRef {
    r(id.as_str())
}
/// A space is a room with `type: m.space`; as a community it keeps its room id.
pub fn space_ref(id: &OwnedRoomId) -> CommunityRef {
    r(id.as_str())
}
pub fn event_ref(id: &OwnedEventId) -> MessageRef {
    r(id.as_str())
}

pub fn user_id(u: &UserRef) -> Result<OwnedUserId, AdapterError> {
    let raw = u.raw_for(Platform::Matrix)?;
    UserId::parse(raw).map_err(|e| AdapterError::Id(e.to_string()))
}

/// `space` is the bot's configured space for this room, if any.
pub fn place(room: &OwnedRoomId, space: Option<&OwnedRoomId>, thread_root: Option<&OwnedEventId>) -> Place {
    Place {
        community: space.map(space_ref),
        room: room_ref(room),
        thread: thread_root.map(|e| r::<transpose_core::ids::Thread>(e.as_str()) as ThreadRef),
    }
}

/// Power levels → standing. Admin: a v12 creator, or power to change the power
/// levels themselves. Moderator: power to kick.
pub fn standing(pl: &RoomPowerLevels, user: &UserId) -> Standing {
    let power = match pl.for_user(user) {
        UserPowerLevel::Infinite => Level::Infinite,
        UserPowerLevel::Int(i) => Level::Int(i.into()),
        _ => Level::Int(0),
    };
    Standing {
        admin: power == Level::Infinite || pl.user_can_send_state(user, StateEventType::RoomPowerLevels),
        moderator: pl.user_can_kick(user),
        roles: Default::default(),
        power: Some(power),
    }
}

pub fn mention_html(u: &UserRef) -> String {
    let id = u.raw().as_str();
    format!("<a href=\"https://matrix.to/#/{id}\">{id}</a>")
}
fn mention_plain(u: &UserRef) -> String {
    u.raw().as_str().to_string()
}

/// What the bot sends: the message, then one `m.reaction` per action emoji.
pub struct Rendered {
    pub content: RoomMessageEventContent,
    pub reactions: Vec<String>,
}

/// A card as an `m.notice` (bots should not send `m.text`) with HTML and a plain body.
pub fn render(card: &Card) -> Rendered {
    let mut rich: Vec<Span> = Vec::new();
    if let Some(t) = &card.title {
        let title = match &card.url {
            Some(url) => Span::Link {
                label: t.clone(),
                url: url.clone(),
            },
            None => Span::Text(t.clone()),
        };
        rich.push(Span::Bold(vec![title]));
        rich.push(Span::Text("\n".into()));
    }
    rich.extend(card.body.iter().cloned());
    for f in &card.fields {
        rich.push(Span::Text("\n".into()));
        rich.push(Span::Bold(vec![Span::Text(format!("{}: ", f.name))]));
        rich.extend(f.value.iter().cloned());
    }
    if let Some(footer) = &card.footer {
        rich.push(Span::Text("\n".into()));
        rich.push(Span::Italic(vec![Span::Text(footer.clone())]));
    }
    let (reactions, hint) = match action_plan(&CAPS, &card.actions) {
        ActionPlan::Reactions(a) => (a.into_iter().map(|a| a.emoji).collect(), None),
        ActionPlan::Hint(cmds) => (Vec::new(), Some(cmds.join(", "))),
        _ => (Vec::new(), None),
    };
    if let Some(h) = hint {
        rich.push(Span::Text(format!("\n{h}")));
    }
    let html = to_html(&rich, &mention_html).replace('\n', "<br>");
    let plain = to_plain(&rich, &mention_plain);
    Rendered {
        content: RoomMessageEventContent::notice_html(plain, html),
        reactions,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error(transparent)]
    Ref(#[from] RefError),
    #[error("not a Matrix id: {0}")]
    Id(String),
}

// ---- inbound -------------------------------------------------------------

/// A text message → a command. Only `m.text` counts: notices are bot output.
pub fn command(
    content: &RoomMessageEventContent,
    sender: &UserId,
    place: Place,
    specs: &[CommandSpec],
    prefix: &str,
) -> Result<Option<Inbound>, ParseError> {
    let MessageType::Text(t) = &content.msgtype else {
        return Ok(None);
    };
    match parse_text(specs, prefix, &t.body) {
        Ok(p) => Ok(Some(Inbound::Command(Invocation {
            actor: user_ref(sender),
            place,
            command: p.command,
            args: p.args,
        }))),
        Err(ParseError::NotACommand) => Ok(None),
        Err(e) => Err(e),
    }
}

/// An `m.reaction` annotation → an action press, when it is one the bot seeded.
pub fn press(
    reaction: &ReactionEventContent,
    sender: &UserId,
    place: Place,
    pressables: &Pressables,
    bot: &UserId,
) -> Option<Inbound> {
    if sender == bot {
        return None;
    }
    let Annotation { event_id, key, .. } = &reaction.relates_to;
    let message = event_ref(event_id);
    let action = pressables.resolve(&message, key)?;
    Some(Inbound::ActionPressed {
        actor: user_ref(sender),
        place,
        message,
        action: action.to_string(),
    })
}

// ---- voice signalling ----------------------------------------------------

/// The LiveKit JWT service's answer, after the bot has published its MatrixRTC
/// membership and exchanged a Matrix OpenID token. Shape per lk-jwt-service's
/// `/sfu/get`; verify before relying on it.
#[derive(Debug, Clone, Deserialize)]
pub struct SfuResponse {
    pub url: String,
    pub jwt: String,
}

/// Where Matrix signalling ends. Element Call encrypts call media with keys
/// sent over Matrix, so the media layer must take part in that exchange.
pub fn grant(resp: SfuResponse) -> LiveKitGrant {
    LiveKitGrant {
        url: resp.url,
        token: resp.jwt,
        media_e2ee: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruma::events::TimelineEventType;
    use ruma::events::room::power_levels::{RoomPowerLevelsEventContent, RoomPowerLevelsSource};
    use ruma::room_version_rules::AuthorizationRules;
    use ruma::{int, owned_room_id, owned_user_id};
    use transpose_core::authority::{Gate, Presence};
    use transpose_core::bot::{Effect, now_playing, skip};
    use transpose_core::caps::PlatformCaps;
    use transpose_core::place::Scope;
    use transpose_core::settings::Settings;
    use transpose_core::voice::{Queue, Track};

    fn levels(creator: &OwnedUserId, dj: &OwnedUserId) -> RoomPowerLevels {
        let rules = AuthorizationRules::V12;
        let mut content = RoomPowerLevelsEventContent::new(&rules);
        // As homeservers create rooms: changing power levels needs 100. Without
        // this entry it falls back to state_default (50), and a DJ at 50 could
        // rewrite everyone's power, so `standing` would rightly call them admin.
        content
            .events
            .insert(TimelineEventType::RoomPowerLevels, int!(100));
        content.users.insert(dj.clone(), int!(50));
        RoomPowerLevels::new(
            RoomPowerLevelsSource::Original(content),
            &rules,
            [creator.clone()],
        )
    }

    #[test]
    fn v12_creators_are_admins_and_kick_power_is_moderator() {
        let creator = owned_user_id!("@founder:example.org");
        let dj = owned_user_id!("@dj:example.org");
        let pl = levels(&creator, &dj);
        let c = standing(&pl, &creator);
        assert!(c.admin);
        assert_eq!(c.power, Some(Level::Infinite));
        let d = standing(&pl, &dj);
        assert!(!d.admin && d.moderator);
        assert_eq!(d.power, Some(Level::Int(50)));
        let nobody = standing(&pl, &owned_user_id!("@n:example.org"));
        assert!(!nobody.admin && !nobody.moderator);
    }

    #[test]
    fn a_room_without_a_space_scopes_settings_to_the_room() {
        let room = owned_room_id!("!music:example.org");
        assert_eq!(place(&room, None, None).scope(), Scope::Room(room_ref(&room)));
    }

    #[test]
    fn role_gates_are_refused_and_power_gates_work() {
        let role = transpose_core::ids::RoleRef::new(Platform::Discord, "7").unwrap();
        assert!(Gate::Role(role).check(&CAPS).is_err());
        let creator = owned_user_id!("@founder:example.org");
        let dj = owned_user_id!("@dj:example.org");
        let pl = levels(&creator, &dj);
        let room = owned_room_id!("!music:example.org");
        let mut s = Settings::defaults(Scope::Room(room_ref(&room)));
        s.dj = Gate::MinPower(50);
        s.check(&CAPS).unwrap();
        let u = user_ref(&dj);
        let mut q = Queue::default();
        q.enqueue(Track {
            title: "x".into(),
            url: "u".into(),
            duration_secs: None,
            requested_by: u.clone(),
        });
        assert!(matches!(
            skip(&u, &standing(&pl, &dj), &Presence::default(), &s, &mut q),
            Effect::PlayNext(_)
        ));
        let n = owned_user_id!("@n:example.org");
        assert!(matches!(
            skip(
                &user_ref(&n),
                &standing(&pl, &n),
                &Presence::default(),
                &s,
                &mut q
            ),
            Effect::Reply(_)
        ));
    }

    #[test]
    fn the_now_playing_card_becomes_html_and_reactions() {
        let u = user_ref(&owned_user_id!("@alice:example.org"));
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
        let out = render(&now_playing(q.now.as_ref().unwrap(), &q));
        assert_eq!(out.reactions, vec!["⏯", "⏭", "⏹"]);
        let json = serde_json::to_value(&out.content).unwrap();
        assert_eq!(json["msgtype"], "m.notice");
        let html = json["formatted_body"].as_str().unwrap();
        assert!(
            html.contains("<a href=\"https://matrix.to/#/@alice:example.org\">"),
            "{html}"
        );
        assert!(
            json["body"]
                .as_str()
                .unwrap()
                .contains("Requested by: @alice:example.org")
        );
        let _ = PlatformCaps::TELEGRAM;
    }

    #[test]
    fn text_commands_and_seeded_reactions_become_events() {
        use transpose_core::command::CommandSpec;
        let room = owned_room_id!("!music:example.org");
        let alice = owned_user_id!("@alice:example.org");
        let bot = owned_user_id!("@crack:example.org");
        let specs = vec![CommandSpec {
            name: "skip".into(),
            summary: "Skip".into(),
            args: vec![],
        }];
        let typed = RoomMessageEventContent::text_plain("!skip");
        let got = command(&typed, &alice, place(&room, None, None), &specs, "!").unwrap();
        assert!(matches!(got, Some(Inbound::Command(ref i)) if i.command == "skip"));
        let notice = RoomMessageEventContent::notice_plain("!skip");
        assert!(
            command(&notice, &alice, place(&room, None, None), &specs, "!")
                .unwrap()
                .is_none()
        );

        let sent = ruma::owned_event_id!("$np:example.org");
        let mut presses = Pressables::default();
        presses.remember(
            event_ref(&sent),
            &[transpose_core::render::Action {
                id: "skip".into(),
                label: "Skip".into(),
                emoji: "⏭".into(),
                command: "skip".into(),
            }],
        );
        let react = ReactionEventContent::new(Annotation::new(sent.clone(), "⏭".into()));
        let got = press(&react, &alice, place(&room, None, None), &presses, &bot);
        assert!(matches!(got, Some(Inbound::ActionPressed { ref action, .. }) if action == "skip"));
        assert!(press(&react, &bot, place(&room, None, None), &presses, &bot).is_none());
    }

    #[test]
    fn matrix_signalling_ends_in_an_encrypted_livekit_grant() {
        let resp: SfuResponse = serde_json::from_str(r#"{"url":"wss://sfu.example.org","jwt":"j"}"#).unwrap();
        let g = grant(resp);
        assert!(g.media_e2ee);
        assert_eq!(g.token, "j");
    }
}
