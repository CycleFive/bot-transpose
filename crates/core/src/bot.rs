//! Bot logic written once, against neutral types only. The adapters' tests run
//! these same functions on all three platforms, which is the usability check:
//! the core reads like ordinary domain code.

use crate::authority::{may_control, Decision, Presence, Standing};
use crate::event::{Inbound, Invocation};
use crate::ids::UserRef;
use crate::render::{text, Action, Card, Field, Out, OutBody, Span, Visibility};
use crate::settings::Settings;
use crate::voice::{Queue, Track};

/// The now-playing card. The same value goes to every adapter.
pub fn now_playing(track: &Track, queue: &Queue) -> Card {
    let mut fields = vec![Field {
        name: "Requested by".into(),
        value: vec![Span::Mention(track.requested_by.clone())],
        inline: true,
    }];
    if let Some(next) = queue.next.front() {
        fields.push(Field {
            name: "Up next".into(),
            value: vec![text(next.title.clone())],
            inline: true,
        });
    }
    Card {
        title: Some(track.title.clone()),
        url: Some(track.url.clone()),
        body: vec![Span::Bold(vec![text("Now playing")])],
        fields,
        thumbnail: None,
        footer: Some(format!("{} in queue", queue.next.len())),
        accent: Some(0x1db954),
        actions: vec![
            action("pause", "Pause", "⏯", "pause"),
            action("skip", "Skip", "⏭", "skip"),
            action("stop", "Stop", "⏹", "stop"),
        ],
    }
}

fn action(id: &str, label: &str, emoji: &str, command: &str) -> Action {
    Action {
        id: id.into(),
        label: label.into(),
        emoji: emoji.into(),
        command: command.into(),
    }
}

/// What the bot does with one inbound event.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Reply(Out),
    PlayNext(Option<Track>),
}

/// Skip, from a command or a button: one decision for every platform.
pub fn skip(
    actor: &UserRef,
    standing: &Standing,
    presence: &Presence,
    settings: &Settings,
    queue: &mut Queue,
) -> Effect {
    match may_control(actor, standing, presence, &settings.dj) {
        Decision::Allow => Effect::PlayNext(queue.advance().cloned()),
        Decision::Deny(why) => Effect::Reply(Out {
            body: OutBody::Text {
                text: vec![text(format!("You can't skip right now ({why:?})."))],
            },
            visibility: Visibility::InvokerIfPossible,
        }),
    }
}

/// Route an inbound event. Pressing "skip" and typing `!skip` arrive as
/// different events and meet here.
pub fn handle(
    event: &Inbound,
    standing: &Standing,
    presence: &Presence,
    settings: &Settings,
    queue: &mut Queue,
) -> Option<Effect> {
    let actor = match event {
        Inbound::ActionPressed { actor, action, .. } if action == "skip" => actor,
        Inbound::Command(Invocation { actor, command, .. }) if command == "skip" => actor,
        _ => return None,
    };
    Some(skip(actor, standing, presence, settings, queue))
}
