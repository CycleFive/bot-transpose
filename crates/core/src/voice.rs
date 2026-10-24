//! Voice, split in two: the queue is the bot's, and a transport only plays the
//! one source it is handed.
//!
//! cracktunes keeps its queue inside songbird's `Call` (`TrackQueue` plus
//! `TrackHandle` data). That is the main knot between the music logic and
//! Discord. Here `Queue` is core state, and the transport reports `Ended`, so core
//! decides what plays next.
//!
//! A transport is itself two layers:
//! - **Signalling** (per platform): how the bot gets into the room. Discord does
//!   it with a voice-state update on the gateway. Matrix uses a MatrixRTC member
//!   event, then an OpenID token exchanged at the LiveKit JWT service. Stoat uses
//!   `POST /channels/{id}/join_call`.
//! - **Media**: songbird (Discord's UDP, Opus and DAVE E2EE) or a LiveKit room.
//!   Matrix and Stoat share the LiveKit media layer.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::ids::{RoomRef, UserRef};

/// A track as the bot knows it. Where the audio comes from is core's business;
/// how it reaches the room is the transport's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub title: String,
    pub url: String,
    pub duration_secs: Option<u32>,
    pub requested_by: UserRef,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    pub now: Option<Track>,
    pub next: VecDeque<Track>,
    pub history: Vec<Track>,
}

impl Queue {
    pub fn enqueue(&mut self, t: Track) {
        self.next.push_back(t);
    }

    /// The current track ended or was skipped: move on, and say what to play.
    pub fn advance(&mut self) -> Option<&Track> {
        if let Some(done) = self.now.take() {
            self.history.push(done);
        }
        self.now = self.next.pop_front();
        self.now.as_ref()
    }
}

/// What a transport is asked to play: a URL or local path core has resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSource {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceEvent {
    Ended,
    Failed {
        reason: String,
    },
    Disconnected,
    /// Someone joined or left the bot's room (for auto-pause on an empty room).
    Listeners {
        present: Vec<UserRef>,
    },
}

/// One platform's way into voice. Async in a real bot; plain here, because the
/// proof of concept checks shapes, not I/O.
pub trait VoiceTransport {
    type Session: VoiceSession;
    type Error: std::error::Error + Send + Sync + 'static;
    fn join(&mut self, room: &RoomRef) -> Result<Self::Session, Self::Error>;
}

pub trait VoiceSession {
    type Error: std::error::Error + Send + Sync + 'static;
    fn play(&mut self, source: AudioSource) -> Result<(), Self::Error>;
    fn stop(&mut self) -> Result<(), Self::Error>;
    fn leave(self) -> Result<(), Self::Error>;
}

/// Where a LiveKit-based platform's signalling ends: the room URL and the
/// access token the bot presents. Matrix and Stoat both produce this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveKitGrant {
    pub url: String,
    pub token: String,
    /// Matrix calls can be end-to-end encrypted with keys sent over Matrix;
    /// the media layer must then take keys from signalling. Stoat: none known.
    pub media_e2ee: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::Platform;

    #[test]
    fn advance_keeps_history_and_ends_cleanly() {
        let u = UserRef::new(Platform::Discord, "1").unwrap();
        let t = |n: &str| Track {
            title: n.into(),
            url: format!("https://x/{n}"),
            duration_secs: None,
            requested_by: u.clone(),
        };
        let mut q = Queue::default();
        q.enqueue(t("a"));
        q.enqueue(t("b"));
        assert_eq!(q.advance().unwrap().title, "a");
        assert_eq!(q.advance().unwrap().title, "b");
        assert!(q.advance().is_none());
        assert_eq!(q.history.len(), 2);
    }
}
