//! Platform-neutral types for a community music bot.
//!
//! This crate depends on no chat platform. The workspace enforces it: only the
//! adapter crates (`transpose-discord`, `transpose-matrix`, `transpose-stoat`)
//! depend on serenity, ruma or rive-models. See `docs/design.md`.

pub mod authority;
pub mod bot;
pub mod caps;
pub mod command;
pub mod event;
pub mod ids;
pub mod place;
pub mod render;
pub mod settings;
pub mod voice;
