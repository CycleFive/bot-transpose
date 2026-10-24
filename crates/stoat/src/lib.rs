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

use rive_models::channel::Channel;
use rive_models::embed::SendableEmbed;
use rive_models::message::Interactions;
use rive_models::permission::Permission;
use transpose_core::authority::Standing;
use transpose_core::caps::{
    ActionSurface, AuthorityModel, CommandSurface, EmbedLevel, PlatformCaps, PrivateReply,
    VoiceSupport,
};
use transpose_core::ids::{Kind, Platform, Ref, RefError, UserRef};
use transpose_core::place::Place;
use transpose_core::render::{action_plan, to_markdown, ActionPlan, Card};

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
        Channel::TextChannel { id, server, .. } | Channel::VoiceChannel { id, server, .. } => (id, Some(server)),
        Channel::DirectMessage { id, .. } | Channel::Group { id, .. } | Channel::SavedMessages { id, .. } => (id, None),
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
        roles: roles.iter().map(|r| to_ref::<transpose_core::ids::Role>(r)).collect::<Result<_, _>>()?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use transpose_core::authority::{Gate, Presence};
    use transpose_core::bot::{handle, now_playing, Effect};
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
            q.enqueue(Track { title: n.into(), url: format!("https://youtu.be/{n}"), duration_secs: None, requested_by: u.clone() });
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
        let place = Place { community: Some(to_ref(SERVER).unwrap()), room: room.clone(), thread: None };
        let mut s = Settings::defaults(place.scope());
        s.dj = Gate::Moderator;
        s.check(&CAPS).unwrap();
        let mut q = Queue::default();
        q.enqueue(Track { title: "a".into(), url: "u".into(), duration_secs: None, requested_by: u.clone() });
        let pressed = Inbound::ActionPressed {
            actor: u.clone(),
            place: place.clone(),
            message: to_ref::<transpose_core::ids::Message>("01HSV4X").map(|m: MessageRef| m).unwrap(),
            action: "skip".into(),
        };
        let member = standing(Permission::SendMessage, &[], false).unwrap();
        let modr = standing(Permission::ManageMessages, &[], false).unwrap();
        assert!(matches!(handle(&pressed, &member, &Presence::default(), &s, &mut q), Some(Effect::Reply(_))));
        assert!(matches!(handle(&pressed, &modr, &Presence::default(), &s, &mut q), Some(Effect::PlayNext(_))));
        assert!(matches!(s.scope, Scope::Community(_)));
        let _: RoleRef = to_ref("01HROLE").unwrap();
    }
}
