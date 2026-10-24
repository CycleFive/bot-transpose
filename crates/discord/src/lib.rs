//! Discord adapter: serenity's types in, neutral types out, and back.
//!
//! Conversions are extension traits defined here, not `From` impls: both
//! serenity's ids and core's refs are foreign to this crate, so the orphan
//! rule forbids `impl From<GuildId> for CommunityRef`. That is a feature: the
//! only place a Discord id becomes a ref is this crate.

use serenity::all::{
    ButtonStyle, ChannelId, Colour, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter,
    GuildId, Permissions, ReactionType, RoleId, UserId,
};
use transpose_core::authority::Standing;
use transpose_core::caps::{
    ActionSurface, AuthorityModel, CommandSurface, EmbedLevel, PlatformCaps, PrivateReply,
    VoiceSupport,
};
use transpose_core::ids::{CommunityRef, Platform, RefError, RoleRef, RoomRef, UserRef};
use transpose_core::place::Place;
use transpose_core::render::{action_plan, to_markdown, ActionPlan, Card};

pub const CAPS: PlatformCaps = PlatformCaps {
    platform: Platform::Discord,
    actions: ActionSurface::Buttons,
    embeds: EmbedLevel::Rich,
    private_reply: PrivateReply::Full,
    commands: CommandSurface::Registered { autocomplete: true },
    edits: true,
    reactions: true,
    threads: true,
    authority: AuthorityModel::RolesAndPermissions,
    voice: VoiceSupport::Native,
    max_text: 2000,
};

/// Discord id → neutral ref. Snowflakes always make valid raw ids.
pub trait ToRef {
    type Ref;
    fn to_ref(self) -> Self::Ref;
}

macro_rules! to_ref {
    ($($id:ty => $r:ty),*) => {$(
        impl ToRef for $id {
            type Ref = $r;
            fn to_ref(self) -> $r {
                <$r>::new(Platform::Discord, self.get().to_string()).expect("a snowflake is a valid raw id")
            }
        }
    )*};
}
to_ref!(GuildId => CommunityRef, ChannelId => RoomRef, UserId => UserRef, RoleId => RoleRef);

/// Neutral ref → Discord id, refusing refs from other platforms.
pub fn snowflake<K: transpose_core::ids::Kind>(r: &transpose_core::ids::Ref<K>) -> Result<u64, AdapterError> {
    let raw = r.raw_for(Platform::Discord)?;
    raw.parse().map_err(|_| AdapterError::NotASnowflake(raw.to_string()))
}

pub fn guild_id(r: &CommunityRef) -> Result<GuildId, AdapterError> {
    Ok(GuildId::new(snowflake(r)?))
}

pub fn place(guild: Option<GuildId>, channel: ChannelId) -> Place {
    Place {
        community: guild.map(ToRef::to_ref),
        room: channel.to_ref(),
        thread: None,
    }
}

/// Permission bits and roles → standing. Owner, ADMINISTRATOR and MANAGE_GUILD
/// are admin; MANAGE_MESSAGES or MOVE_MEMBERS is moderator.
pub fn standing(perms: Permissions, roles: &[RoleId], is_owner: bool) -> Standing {
    Standing {
        admin: is_owner || perms.administrator() || perms.manage_guild(),
        moderator: perms.manage_messages() || perms.move_members(),
        roles: roles.iter().map(|r| r.to_ref()).collect(),
        power: None,
    }
}

pub fn mention(u: &UserRef) -> String {
    format!("<@{}>", u.raw().as_str())
}

/// A card as an embed plus button rows. Discord renders everything the card has.
pub fn render(card: &Card) -> (CreateEmbed, Vec<CreateActionRow>) {
    let mut e = CreateEmbed::new().description(to_markdown(&card.body, &mention));
    if let Some(t) = &card.title {
        e = e.title(t);
    }
    if let Some(u) = &card.url {
        e = e.url(u);
    }
    for f in &card.fields {
        e = e.field(&f.name, to_markdown(&f.value, &mention), f.inline);
    }
    if let Some(t) = &card.thumbnail {
        e = e.thumbnail(t);
    }
    if let Some(f) = &card.footer {
        e = e.footer(CreateEmbedFooter::new(f));
    }
    if let Some(c) = card.accent {
        e = e.colour(Colour::new(c));
    }
    let rows = match action_plan(&CAPS, &card.actions) {
        ActionPlan::Buttons(actions) => actions
            .chunks(5)
            .map(|chunk| {
                CreateActionRow::Buttons(
                    chunk
                        .iter()
                        .map(|a| {
                            CreateButton::new(&a.id)
                                .label(&a.label)
                                .emoji(ReactionType::Unicode(a.emoji.clone()))
                                .style(ButtonStyle::Secondary)
                        })
                        .collect(),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    (e, rows)
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error(transparent)]
    Ref(#[from] RefError),
    #[error("{0:?} is not a Discord snowflake")]
    NotASnowflake(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use transpose_core::authority::{Gate, Presence};
    use transpose_core::bot::{now_playing, skip, Effect};
    use transpose_core::place::Scope;
    use transpose_core::settings::Settings;
    use transpose_core::voice::{Queue, Track};

    #[test]
    fn ids_round_trip_and_refuse_other_platforms() {
        let g = GuildId::new(80351110224678912);
        let r = g.to_ref();
        assert_eq!(r.key(), "discord:80351110224678912");
        assert_eq!(guild_id(&r).unwrap(), g);
        let matrix = CommunityRef::new(Platform::Matrix, "!space:example.org").unwrap();
        assert!(guild_id(&matrix).is_err());
    }

    fn queue() -> (UserRef, Queue) {
        let u = UserId::new(42).to_ref();
        let t = |n: &str| Track {
            title: n.into(),
            url: format!("https://youtu.be/{n}"),
            duration_secs: Some(212),
            requested_by: u.clone(),
        };
        let mut q = Queue::default();
        q.enqueue(t("Song A"));
        q.enqueue(t("Song B"));
        q.advance();
        (u, q)
    }

    #[test]
    fn the_now_playing_card_becomes_an_embed_with_buttons() {
        let (_, q) = queue();
        let (embed, rows) = render(&now_playing(q.now.as_ref().unwrap(), &q));
        let json = serde_json::to_value(&embed).unwrap();
        assert_eq!(json["title"], "Song A");
        assert_eq!(json["fields"][0]["value"], "<@42>");
        assert_eq!(json["fields"][1]["value"], "Song B");
        let rows = serde_json::to_value(&rows).unwrap();
        assert_eq!(rows[0]["components"].as_array().unwrap().len(), 3);
        assert_eq!(rows[0]["components"][1]["custom_id"], "skip");
    }

    #[test]
    fn a_dj_role_gate_works_through_discord_roles() {
        let (u, mut q) = queue();
        let dj = RoleId::new(7);
        let mut s = Settings::defaults(Scope::Community(GuildId::new(1).to_ref()));
        s.dj = Gate::Role(dj.to_ref());
        s.check(&CAPS).unwrap();
        let without = standing(Permissions::empty(), &[], false);
        assert!(matches!(skip(&u, &without, &Presence::default(), &s, &mut q), Effect::Reply(_)));
        let with = standing(Permissions::empty(), &[dj], false);
        assert!(matches!(skip(&u, &with, &Presence::default(), &s, &mut q), Effect::PlayNext(_)));
    }
}
