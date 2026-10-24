//! What the bot wants to say, before any platform shapes it.
//!
//! Today's cracktunes `Rendered` holds serenity's `CreateEmbed` and
//! `CreateComponent` builders. Here the intent is data: rich text as a small
//! tree, a `Card` for embed-like output, and `Action`s for buttons. Each adapter
//! renders it within its `PlatformCaps`, and degrades in a known order:
//! buttons, then reactions, then a text hint.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::caps::{ActionSurface, PlatformCaps};
use crate::ids::{MessageRef, UserRef};

/// Inline rich text. Mentions are refs, never pre-formatted strings:
/// `<@123>` on Discord, a matrix.to pill on Matrix, `<@01H…>` on Stoat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum Span {
    Text(String),
    Bold(Vec<Span>),
    Italic(Vec<Span>),
    Code(String),
    Link { label: String, url: String },
    Mention(UserRef),
}

pub type Rich = Vec<Span>;

pub fn text(s: impl Into<String>) -> Span {
    Span::Text(s.into())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub title: Option<String>,
    pub url: Option<String>,
    pub body: Rich,
    pub fields: Vec<Field>,
    pub thumbnail: Option<String>,
    pub footer: Option<String>,
    /// RGB.
    pub accent: Option<u32>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub value: Rich,
    pub inline: bool,
}

/// A pressable action. `id` comes back in `Inbound::ActionPressed`; `emoji` is
/// the press on reaction platforms; `command` is the text hint fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub label: String,
    pub emoji: String,
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Everyone,
    /// Shown to the invoker only where the platform can; otherwise everyone.
    InvokerIfPossible,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Out {
    pub body: OutBody,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutBody {
    Text { text: Rich },
    Card { card: Card },
}

/// How a platform will present a card's actions. Adapters call this rather than
/// deciding for themselves, so every platform degrades the same way.
pub fn action_plan(caps: &PlatformCaps, actions: &[Action]) -> ActionPlan {
    if actions.is_empty() {
        return ActionPlan::None;
    }
    match caps.actions {
        ActionSurface::Buttons => ActionPlan::Buttons(actions.to_vec()),
        ActionSurface::Reactions if caps.reactions => ActionPlan::Reactions(actions.to_vec()),
        _ => ActionPlan::Hint(actions.iter().map(|a| a.command.clone()).collect()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionPlan {
    None,
    Buttons(Vec<Action>),
    Reactions(Vec<Action>),
    Hint(Vec<String>),
}

/// On reaction platforms a press arrives as "user reacted with ⏭ on message M",
/// so the bot must remember what each of its messages offered. Button platforms
/// carry the action id in the press and never need this.
#[derive(Debug, Default)]
pub struct Pressables {
    by_message: HashMap<MessageRef, Vec<(String, String)>>,
}

impl Pressables {
    pub fn remember(&mut self, message: MessageRef, actions: &[Action]) {
        let pairs = actions.iter().map(|a| (a.emoji.clone(), a.id.clone())).collect();
        self.by_message.insert(message, pairs);
    }

    /// The action id for a reaction, or None for a reaction that is not a press
    /// (someone's own emoji, or a message the bot has forgotten).
    pub fn resolve(&self, message: &MessageRef, emoji: &str) -> Option<&str> {
        self.by_message
            .get(message)?
            .iter()
            .find(|(e, _)| e == emoji)
            .map(|(_, id)| id.as_str())
    }

    /// Call when the message is replaced or deleted, so the index stays small.
    pub fn forget(&mut self, message: &MessageRef) {
        self.by_message.remove(message);
    }
}

/// Markdown in the shape Discord and Stoat both read. Mentions use the
/// adapter's spelling.
pub fn to_markdown(rich: &[Span], mention: &dyn Fn(&UserRef) -> String) -> String {
    let mut out = String::new();
    for s in rich {
        match s {
            Span::Text(t) => out.push_str(&escape_md(t)),
            Span::Bold(k) => {
                out.push_str("**");
                out.push_str(&to_markdown(k, mention));
                out.push_str("**");
            }
            Span::Italic(k) => {
                out.push('*');
                out.push_str(&to_markdown(k, mention));
                out.push('*');
            }
            Span::Code(c) => {
                out.push('`');
                out.push_str(&c.replace('`', "'"));
                out.push('`');
            }
            Span::Link { label, url } => out.push_str(&format!("[{}]({url})", escape_md(label))),
            Span::Mention(u) => out.push_str(&mention(u)),
        }
    }
    out
}

/// HTML in the subset Matrix's `formatted_body` and Telegram's HTML mode share.
pub fn to_html(rich: &[Span], mention: &dyn Fn(&UserRef) -> String) -> String {
    let mut out = String::new();
    for s in rich {
        match s {
            Span::Text(t) => out.push_str(&escape_html(t)),
            Span::Bold(k) => out.push_str(&format!("<b>{}</b>", to_html(k, mention))),
            Span::Italic(k) => out.push_str(&format!("<i>{}</i>", to_html(k, mention))),
            Span::Code(c) => out.push_str(&format!("<code>{}</code>", escape_html(c))),
            Span::Link { label, url } => out.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                escape_html(url),
                escape_html(label)
            )),
            Span::Mention(u) => out.push_str(&mention(u)),
        }
    }
    out
}

/// Plain text, for Matrix's required `body` and for logs.
pub fn to_plain(rich: &[Span], mention: &dyn Fn(&UserRef) -> String) -> String {
    let mut out = String::new();
    for s in rich {
        match s {
            Span::Text(t) | Span::Code(t) => out.push_str(t),
            Span::Bold(k) | Span::Italic(k) => out.push_str(&to_plain(k, mention)),
            Span::Link { label, url } => out.push_str(&format!("{label} ({url})")),
            Span::Mention(u) => out.push_str(&mention(u)),
        }
    }
    out
}

fn escape_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '_' | '`' | '~' | '|' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::PlatformCaps;
    use crate::ids::Platform;

    fn skip() -> Action {
        Action {
            id: "skip".into(),
            label: "Skip".into(),
            emoji: "⏭".into(),
            command: "!skip".into(),
        }
    }

    #[test]
    fn a_reaction_resolves_only_on_the_message_that_offered_it() {
        let m1 = MessageRef::new(Platform::Stoat, "M1").unwrap();
        let m2 = MessageRef::new(Platform::Stoat, "M2").unwrap();
        let mut p = Pressables::default();
        p.remember(m1.clone(), &[skip()]);
        assert_eq!(p.resolve(&m1, "⏭"), Some("skip"));
        assert_eq!(p.resolve(&m1, "🎉"), None);
        assert_eq!(p.resolve(&m2, "⏭"), None);
        p.forget(&m1);
        assert_eq!(p.resolve(&m1, "⏭"), None);
    }

    #[test]
    fn telegram_has_buttons() {
        assert!(matches!(
            action_plan(&PlatformCaps::TELEGRAM, &[skip()]),
            ActionPlan::Buttons(_)
        ));
    }

    #[test]
    fn markup_is_escaped_in_both_dialects() {
        let rich = vec![Span::Bold(vec![text("a*b <c>")])];
        let none = |_: &UserRef| String::new();
        assert_eq!(to_markdown(&rich, &none), "**a\\*b <c>**");
        assert_eq!(to_html(&rich, &none), "<b>a*b &lt;c&gt;</b>");
    }

    #[test]
    fn mentions_use_the_adapters_spelling() {
        let u = UserRef::new(Platform::Discord, "42").unwrap();
        let rich = vec![text("hi "), Span::Mention(u)];
        let discord = |u: &UserRef| format!("<@{}>", u.raw().as_str());
        assert_eq!(to_markdown(&rich, &discord), "hi <@42>");
    }
}
