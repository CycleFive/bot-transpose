# bot-transpose

One bot, several chat platforms. This repo holds platform-neutral types for a community music
bot, and how they map onto **Discord**, **Matrix** and **Stoat** (formerly Revolt), with
**Telegram** as a capability stress test.

It is a design exercise with proof-of-concept code, held apart from
[cracktunes](https://github.com/cycle-five/cracktunes) on purpose: work out what the model
looks like first, then decide what cracktunes adopts. Start with **[`docs/design.md`](docs/design.md)**.

## Layout

| crate | depends on | what it holds |
|---|---|---|
| `transpose-core` | serde only | refs, places and scopes, authority (`Standing`, `Gate`), output (`Card`, `Action`, degradation), commands, the core-owned queue and the voice split, storage row shapes, and bot logic written once (`bot.rs`) |
| `transpose-web` | core | what a dashboard sees: `SettingsView`, `SettingsPatch` validated against the platform, `QueueView`, and the `Directory` trait for names |
| `transpose-discord` | serenity 0.12 | ids, standing, embed and buttons, button presses |
| `transpose-matrix` | ruma 0.17 | ids, power levels (room v12 creators included), HTML and reactions, `m.text` commands and `m.reaction` presses, MatrixRTC grant |
| `transpose-stoat` | rive-models 1.2 | ids, permissions, simple embed and pinned reactions, commands and reaction presses, replies, `join_call` grant, a `Directory` for the dashboard |

`docs/schema.sql` shows the storage side: the `(platform, raw)` column pair, a typed
`platform` enum, and a lifecycle for extending and deprecating platforms.

## What is decided

- **Text ids**, typed by role (`UserRef`, `RoomRef`, …) and tagged by platform.
- **Stored as a `(platform, raw)` column pair.** JSON mirrors it as `{"platform": "…", "id": "…"}`.
- **The dashboard speaks the neutral types too.**
- **Stoat next, then Matrix.**

## Running

```sh
cargo test --workspace          # 43 tests: the same bot logic through every adapter
cargo tree -p transpose-core    # no serenity, ruma or rive-models
cargo tree -p transpose-web     # likewise
```

CI runs rustfmt, clippy with `-D warnings`, the tests, and a check that core and web depend on
no chat platform.

## Status

This is a proof of concept. It has no network I/O, no real voice, no live database and no
running bot. Two shapes are our own reading of third-party sources and need checking before
anyone relies on them: Stoat's `join_call` response and lk-jwt-service's `/sfu/get` answer.

MIT licensed.
