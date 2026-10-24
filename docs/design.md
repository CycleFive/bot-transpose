# transpose: one bot, several platforms

A design exercise, kept apart from cracktunes, on purpose. It asks one question:
what types would a community music bot have if Discord were one platform among
several, and how do they map onto Discord, Matrix and a third Discord-like
platform? The answer is meant to inform what cracktunes does next, without
committing it to anything.

> "let's design the types, our general types, and how they map to discord and
> matrix's, and maybe telegram's … inner types … work out what that would look like
> first, then we say okay, what can we do here and is it what we want to do."
> (the owner, 2026-10-24)

The proof of concept in `crates/` compiles every mapping in this document against
the real platform crates (serenity 0.12.5, ruma 0.17.2, rive-models 1.2.1) and
runs the same bot logic through all three. Its code is illustration; the
structure is the point.

## Decisions so far (the owner, 2026-10-24)

| Question | Decision | Where it shows |
|---|---|---|
| How ids are represented | **Text**, everywhere | `RawId(Arc<str>)` in §4.1 |
| How they are stored | **A `(platform, raw)` column pair**, so the platform is a typed column that can be extended or deprecated | §4.9, `docs/schema.sql`, `Lifecycle` |
| Does the dashboard use the neutral types? | **Yes.** crack-web should speak refs too | §4.10, `crates/web` |
| Which platform comes second | **Stoat first, then Matrix** | the Stoat adapter is the most complete (§5) |
| What cracktunes does next | **Steps 1–3 of §6**, worth doing even on Discord alone | §6 |

## 1. Why now

The topology map of cracktunes (ct#632) measured what "Discord-specific" means in
practice:
- 79% of crack-core's non-test lines carry Discord in their signatures.
- Most of it comes straight from Discord's ID types (`GuildId` in 553 units,
  `UserId` 349, `ChannelId` 182, `GenericChannelId` 160).
- The rest flows through a few carrier types: `Context`, `CrackedError`,
  `DataInner`, `GuildSettings` and `Rendered`.

The cheapest cuts the map found are the ID types and `GuildSettings`, whose only
Discord tie is one `GuildId` field.

Reading `GuildSettings` while writing this turned up something the map cannot
see. Besides that field, it stores Discord ids as bare `u64`: `authorized_users`,
`authorized_groups` and `ignored_channels`. A `u64` is not a Discord type, so
no tool flags it, but the data is Discord-shaped all the same. Neutral id types
fix both the visible and the invisible kind.

Matrix is not hypothetical. The owner said CrackTunes would have real users there,
music first. The 2026-10-22 investigation (`docs/investigations/2026-10-22-matrix-music.md`
in cracktunes) found it possible but not now, because the queue lives inside songbird.

## 2. The platforms

| | Discord | Matrix | Stoat (ex-Revolt) | Telegram (stress test) |
|---|---|---|---|---|
| Status, Oct 2026 | the platform today | federated, open | open source, self-hostable; renamed from Revolt in Oct 2025 | huge, but bots cannot join voice |
| Ids | u64 snowflakes | `@user:server`, `!room:server` (room v12 room ids drop the server part), `$event` | 26-char ULID strings | i64; group chat ids are negative |
| Community | guild | space (a room with `type: m.space`); a room can be in **several** or none | server | none (a group is a chat) |
| Where chat happens | channel; thread | room; thread (`m.thread` relation) | channel | chat; forum topic |
| Voice place | voice / stage channel | any room can hold a call | voice channel | group voice chat |
| Authority | permission bits + roles + channel overwrites | power levels per room (integers; v12 creators are "infinite") | permission bits + ranked roles + channel overrides | admin rights per member |
| Output | markdown, rich embeds with fields, buttons and selects | `m.notice` with plain `body` + HTML `formatted_body`; no buttons; reactions; polls | markdown; embeds without fields; pinned reactions (`interactions`) | HTML or MarkdownV2; inline-keyboard buttons; no embeds |
| Private reply | ephemeral interaction replies | none | none | a toast on a button press only |
| Commands | registered, typed options, autocomplete | text, by convention `!cmd` | text with a prefix | `/cmd` names in a menu; arguments are text |
| Voice transport | voice gateway + UDP + Opus, DAVE E2EE mandatory since 2026-03-01 | MatrixRTC: member event, OpenID, LiveKit JWT service, then a LiveKit room; media E2EE keys over Matrix | `POST /channels/{id}/join_call` returns a LiveKit URL and token (LiveKit since Feb 2026, per third-party sources) | no: needs a separate user session (MTProto), not a bot token |
| Rust crates | serenity, poise, songbird | ruma, matrix-sdk | rive-models, rive (community) | teloxide |

**Third platform: Stoat.** It is the Discord clone with a future: it's open source,
has Rust model crates, and voice on LiveKit. Guilded closed on 2025-12-19 and
redirects to Roblox Communities. Fluxer is young and its bot API is undocumented.
Telegram stays as a column in the capability tables because it breaks assumptions
in useful ways: it has buttons but no embeds, a toast but no private replies, and
no voice for bots at all.

**The structural finding:** two of the three voice paths end in a LiveKit room.
Voice therefore splits into per-platform signalling over a shared media layer, and
only Discord needs its own media (songbird).

## 3. Principles

1. **The core never sees a platform type, and the build enforces it.**
   `transpose-core` has no dependency on serenity, ruma or rive-models, so an
   import cannot creep in.
2. **IDs are opaque, typed by role, and tagged by platform.** Core compares, stores
   and prints them, and never parses them.
3. **Ask about capabilities, never platforms.** Core asks "are there buttons?",
   not "is this Discord?". Each adapter publishes one `PlatformCaps` constant.
4. **Authority is computed by the adapter and decided by core.** The adapter turns
   bits, roles or power levels into a neutral `Standing`. Settings express rules as
   `Gate`s. The decision is a pure function.
5. **Output is intent; the adapter shapes it.** One `Card` renders as an embed with
   buttons, as HTML plus reactions, or as a simple embed plus pinned reactions.
   Degradation always runs in the same order.
6. **The bot owns its state.** The queue, settings and economy are core types. A
   platform library is a transport, never the home of state; that is the songbird
   knot.
7. **Places are a tree with optional levels.** Community? then room, then thread?
   It is not a fixed guild-and-channel pair.

## 4. The types

Paths are in `crates/core/src/`.

### 4.1 References (`ids.rs`)

```rust
pub struct Ref<K: Kind> { platform: Platform, raw: RawId /* Arc<str> */, .. }
pub type UserRef = Ref<User>;       pub type CommunityRef = Ref<Community>;
pub type RoomRef = Ref<Room>;       pub type ThreadRef = Ref<Thread>;
pub type MessageRef = Ref<Message>; pub type RoleRef = Ref<Role>;
```

- **Role-typed by a phantom marker.** One implementation covers six kinds, and a
  `RoomRef` cannot be passed as a `CommunityRef`. The set of kinds is sealed.
- **One raw representation: text.** Snowflakes, negative Telegram ids, Matrix ids
  and ULIDs all fit, and core never needs the number. The cost is that refs are
  `Clone`, not `Copy`. A clone is a reference-count bump, and core mostly passes
  `&UserRef`. cracktunes code that passes `GuildId` by value today will need a
  `.clone()` or a borrow.
- **Stored as a column pair, `(platform, raw)`.** The platform is a typed column (a
  Postgres enum; §4.9) and the raw id is text. JSON mirrors the pair as
  `{"platform": "stoat", "id": "01HV…"}`, so the dashboard receives the typed
  platform too. A one-line key, `<platform>:<raw>`, exists for logs, config files
  and map keys. The kind is never stored: it comes from the column the pair is in.
- **A platform has a lifecycle.** `Active` platforms mint refs. `Modelled`
  (Telegram: capability tables only, no adapter) and `Deprecated` (had an adapter
  once) refuse `Ref::new`. `Ref::from_stored` still loads them, so old rows stay
  readable after a platform is retired.
- **The only exit is checked.** `Ref::raw_for(platform)` refuses a ref from
  another platform, so a Matrix room can never reach a Discord API call.
- **Adapters convert with extension traits, not `From`.** The orphan rule forbids
  `impl From<GuildId> for CommunityRef` in an adapter crate, since both types are
  foreign there. That rule helps: the only place a `GuildId` becomes a ref is the
  Discord adapter (`ToRef for GuildId`).

Rejected alternative: a `Platform` trait with associated id types, with core code
generic over `P: Platform`. It gives compile-time platform separation, but every
core function becomes generic. Storage needs an erased key anyway, and a process
serving two platforms needs a runtime tag regardless. The runtime tag plus
role typing catches the mistakes that matter (wrong kind, wrong platform) at the
cost of one checked conversion at the edge.

### 4.2 Places and scope (`place.rs`)

```rust
pub struct Place { pub community: Option<CommunityRef>, pub room: RoomRef, pub thread: Option<ThreadRef> }
pub enum Scope { Community(CommunityRef), Room(RoomRef) }   // what settings attach to
```

| | Discord | Matrix | Stoat |
|---|---|---|---|
| `community` | the guild (None in DMs) | the space the bot was told about; None otherwise, since a room may be in several | the server (None in DMs, groups) |
| `room` | the channel | the room | the channel |
| `thread` | the thread; its parent becomes `room` | the thread root event | none |
| `scope()` | the guild | the space if configured, else the room | the server |

`GuildSettings { guild_id, .. }` becomes `Settings { scope, .. }`. On Matrix a bot
in a lone room still has settings, scoped to that room.

### 4.3 Authority (`authority.rs`)

```rust
pub struct Standing { pub admin: bool, pub moderator: bool, pub roles: BTreeSet<RoleRef>, pub power: Option<Level> }
pub enum Level { Int(i64), Infinite }                        // Infinite = Matrix v12 creator
pub enum Gate { Anyone, Moderator, Admin, Role(RoleRef), MinPower(i64), Users(BTreeSet<UserRef>) }
pub fn may_control(actor, &Standing, &Presence, &Gate) -> Decision
```

| `Standing` from | admin | moderator | roles | power |
|---|---|---|---|---|
| Discord | owner, ADMINISTRATOR or MANAGE_GUILD | MANAGE_MESSAGES or MOVE_MEMBERS | member roles | — |
| Matrix | v12 creator, or may send `m.room.power_levels` | at or above the kick level | — | the user's level |
| Stoat | owner or ManageServer | ManageMessages or MoveMembers | member roles | — |
| Telegram | creator, or an admin with promote rights | admin with restrict rights | — | — |

**Structural correctness here means invalid settings can't be saved.**
`Gate::check(&caps)` refuses a `Role` gate on Matrix or Telegram, a `MinPower`
gate on Discord or Stoat, and any gate that names a user from another platform.
A DJ role therefore cannot be configured on a platform with no roles; the Matrix
equivalent is "power level 50 or more".

The PoC found one real subtlety. Matrix admin means "can change the power levels".
If a room's `events` map has no entry for `m.room.power_levels`, that permission
falls back to `state_default` (often 50). A user at 50 can then rewrite everyone's
power, so they really are an admin. The adapter computes this from the room's
actual rules rather than assuming 100.

### 4.4 Output (`render.rs`)

```rust
pub enum Span { Text(String), Bold(Vec<Span>), Italic(Vec<Span>), Code(String), Link{..}, Mention(UserRef) }
pub struct Card { title, url, body: Rich, fields: Vec<Field>, thumbnail, footer, accent, actions: Vec<Action> }
pub struct Action { id, label, emoji, command }        // id ← button; emoji ← reaction; command ← text hint
pub enum Visibility { Everyone, InvokerIfPossible }
```

- **Mentions are refs.** `<@42>` on Discord, a `matrix.to` link on Matrix, and
  `<@01H…>` on Stoat. Core never pre-formats one.
- **One degradation order:** buttons, then reactions, then a text hint, chosen by
  `action_plan(&caps, &actions)`. Adapters call it rather than deciding themselves.
- **The shared formatters are in core because several platforms use each:**
  `to_markdown` (Discord, Stoat), `to_html` (Matrix, Telegram) and `to_plain`
  (Matrix's required `body`).

The now-playing card, written once in `bot.rs`, rendered by the PoC:

| | Discord | Matrix | Stoat |
|---|---|---|---|
| title + url | embed title, url | bold link | embed title, url |
| "Requested by" field | inline embed field `<@42>` | `<b>Requested by: </b><a href="https://matrix.to/#/@alice:…">` | folded into the description: `**Requested by**: <@01H…>` |
| actions | one row of 3 buttons, `custom_id` = `skip` | 3 `m.reaction`s the bot sends | `interactions.reactions` (3), `restrict_reactions: true` |
| accent | embed colour | (none) | `#1db954` |
| msgtype | — | `m.notice` (bots should not send `m.text`) | — |

`InvokerIfPossible` gives an ephemeral reply on Discord, a toast on a Telegram
button press, and an ordinary reply elsewhere. A setting such as
`private_replies` therefore means "where the platform can".

### 4.5 Commands and events (`command.rs`, `event.rs`)

- **Declared once** as `CommandSpec { name, summary, args: Vec<ArgSpec> }`. The
  Discord adapter registers it as an application command. Matrix and Stoat parse
  text with `parse_text(specs, prefix, line)`, which lives in core because three
  platforms share it. The last `Text` argument takes the rest of the line.
- **`Inbound::ActionPressed { action, .. }` carries no hint of how the press
  happened:** a Discord button, a Telegram button, a Matrix reaction or a Stoat
  reaction. In the PoC, a reaction press and a typed `!skip` both reach
  `bot::skip` and are decided the same way (stoat adapter test).
- **Reaction platforms need a memory; button platforms don't.** A Discord button
  carries its action id as `custom_id`. A Stoat or Matrix reaction says only "user
  U reacted ⏭ on message M". `render::Pressables` remembers which emoji meant which
  action on each message the bot sent. It ignores the bot's own seed reactions,
  stray emoji and forgotten messages.

| Inbound, per adapter | Discord | Matrix | Stoat |
|---|---|---|---|
| command | an application-command interaction | `m.text` body through `parse_text` (`m.notice` is ignored: that's bot output) | message `content` through `parse_text` |
| press | `press(custom_id, …)`, no lookup | `m.reaction` annotation through `Pressables` | `MessageReact` event through `Pressables` |

### 4.6 Voice (`voice.rs`)

```rust
pub struct Queue { pub now: Option<Track>, pub next: VecDeque<Track>, pub history: Vec<Track> }  // core-owned
pub trait VoiceTransport { type Session: VoiceSession; fn join(&mut self, room: &RoomRef) -> Result<Self::Session, _>; }
pub trait VoiceSession   { fn play(&mut self, AudioSource) -> ..; fn stop(..); fn leave(self); }
pub struct LiveKitGrant  { pub url: String, pub token: String, pub media_e2ee: bool }  // where LiveKit signalling ends
```

| | signalling | media | notes |
|---|---|---|---|
| Discord | gateway voice-state update, then voice-server update | songbird: UDP, Opus, DAVE (MLS) | DAVE has been mandatory since 2026-03-01; cracktunes' songbird already copes in production |
| Matrix | MatrixRTC member state event, an OpenID token, exchanged at the LiveKit JWT service (`SfuResponse { url, jwt }` → `grant`) | LiveKit room (`livekit` crate) | call media can be E2EE with keys sent over Matrix, so the grant says `media_e2ee: true` and the bot must take part in that key exchange |
| Stoat | `POST /channels/{id}/join_call` (`join_call_path`, then `JoinCallResponse { url, token }` → `grant`) | LiveKit room | rive-models 1.2.1 predates this, so the response shape is ours; check it against stoat.js |
| Telegram | — | — | `VoiceSupport::Unsupported` with a bot token |

The queue is core's, and a transport only plays what it is handed, then reports
`VoiceEvent::Ended`. That reverses cracktunes' current arrangement, where
songbird's `TrackQueue` is the queue.

### 4.7 Capabilities (`caps.rs`)

| `PlatformCaps` | Discord | Matrix | Stoat | Telegram |
|---|---|---|---|---|
| actions | Buttons | Reactions | Reactions | Buttons |
| embeds | Rich | None | Simple | None |
| private_reply | Full | No | No | ToastOnly |
| commands | Registered (autocomplete) | Prefixed | Prefixed | Menu |
| authority | RolesAndPermissions | PowerLevels | RolesAndPermissions | AdminRights |
| voice | Native | LiveKit | LiveKit | Unsupported |
| threads | yes | yes | no | yes (forum topics) |
| max_text | 2,000 | 65,536 (event size) | 2,000 | 4,096 |

### 4.8 Errors (not in the PoC)

`CrackedError` mixes the bot's own failures with serenity's, poise's and songbird's
error types. That is 15 cuts, according to the map. Split it:
- core gets its own error enum, with no platform types in it;
- each adapter gets an error type wrapping its library's errors;
- core sees adapter failures as `PlatformFailure { platform, kind: Retryable |
  Forbidden | NotFound | Other, message }`.

A missing Discord permission becomes `Forbidden`, which core can phrase the same
way everywhere.

### 4.9 Storage (`storage.rs`, `docs/schema.sql`)

Core holds no database crate. `storage.rs` has the plain row shapes a repository
layer binds:
- `RefColumns { platform, raw }` for any ref;
- `ScopeColumns { kind, at }` for a settings scope, stored as
  `(scope_kind, scope_platform, scope_raw)`.

Loading goes through `Ref::from_stored`, so an empty raw id is an error and a
deprecated platform still loads.

`docs/schema.sql` shows the Postgres side:
- a `platform` enum type;
- a `platform_lifecycle` table that mirrors `Platform::lifecycle()`, with a
  trigger that refuses new rows for inactive platforms;
- `settings` keyed by the scope triple;
- child tables for what `GuildSettings` keeps as bare `u64` today
  (authorized users, ignored rooms), each with a check that the ref's platform
  matches the scope's.

**Extending** is a new `Platform` variant plus `ALTER TYPE platform ADD VALUE`
and a lifecycle row. **Deprecating** is `Lifecycle::Deprecated` plus an update to
that row. Enum values are never dropped while rows mention them. With sqlx,
`Platform` would derive `sqlx::Type` against the enum.

### 4.10 The dashboard (`crates/web`)

`transpose-web` is what crack-web would become, and it depends on core only. It
shows the dashboard needs no platform crate either.
- **`Directory`** is a trait adapters implement from their caches to turn refs into
  display names. Lookups are optional, and a missing name falls back to the ref's
  key with `known: false`, so a user who left still shows up.
- **`SettingsView`** carries the scope (with its label), the settings, and the
  platform's `PlatformCaps` with the `gate_options` it allows. The page can then
  offer a role picker on Discord and Stoat, a power-level field on Matrix, and
  neither where no such gate exists.
- **`SettingsPatch`** is what the page sends back. It denies unknown fields (an old
  page posting `guild_id` fails loudly), and `apply` validates the whole result
  with `Settings::check`. A role gate on Matrix, or a Discord user in a Stoat
  server's list, is refused with a typed error before anything is stored.
- **`QueueView`** labels requesters through the same `Directory`.

The Stoat adapter's `StoatDirectory` feeds it from rive-models' `User`, `Server`
and `Channel`, which covers the path from adapter cache to dashboard JSON.

## 5. What the proof of concept shows

- **Platform isolation:** `cargo tree -p transpose-core` and
  `cargo tree -p transpose-web` contain no chat crate.
- **43 tests, clippy and rustfmt clean, CI on every push.** Each adapter runs the
  same `bot::now_playing` and `bot::skip` against its platform's real types:
  - Discord: a DJ role gate through serenity `Permissions` and `RoleId`; a button
    press carries its id.
  - Matrix: a power-level gate through ruma's `RoomPowerLevels`, including a v12
    creator; `m.text` commands and seeded `m.reaction`s become events; signalling
    ends in an encrypted LiveKit grant.
  - Stoat (the next platform, so the most complete):
    - a reaction press, decided by rive-models' `Permission` bits;
    - prefixed commands from `Message`;
    - pinned reactions resolved through `Pressables`;
    - a private reply sent as an ordinary reply in `SendMessageData`;
    - `join_call` signalling into a LiveKit grant;
    - a `StoatDirectory` feeding the dashboard view.
- **Rendering:** the same card renders three ways, asserted on the platform types'
  own serialised JSON: serenity's embed builder, ruma's `RoomMessageEventContent`,
  and rive-models' `SendableEmbed` and `Interactions`.
- **Storage:** refs and scopes round-trip through the column-pair row shapes; the
  JSON form is the same pair.

What it does not show: any I/O, real voice, a live database, or a running bot.

## 6. What this means for cracktunes

The map makes each step measurable. Run `scripts/topology.sh extract` before and
after, then `diff`.

1. **Refs in crack-types.** Add `Ref<K>`, the six kinds and the key encoding.
   Add a Discord `ToRef` for serenity ids. New code uses refs; old code converts at
   the edge. This targets the four id types, which appear as direct evidence 1,244 times
   (553 + 349 + 182 + 160, counting a unit once per type), gradually.
2. **`Settings { scope }`.** Replace `guild_id` with `scope`, and the bare-`u64`
   maps with `BTreeSet<UserRef>`, `BTreeSet<RoleRef>` and `BTreeSet<RoomRef>`.
   This needs a DB migration from bigint ids to the `(platform, raw)` column pair
   (§4.9). It removes `GuildSettings` from the carrier list (sole carrier for 42
   units). crack-web's settings page moves to `SettingsView` and `SettingsPatch` in
   the same step (§4.10), since it reads and writes the same rows.
3. **`Card` in place of `Rendered`.** The Discord adapter's renderer becomes the
   only place `CreateEmbed` and `CreateComponent` appear. `Rendered` carries taint
   into 89 units.
4. **Gates and `Standing`.** The permission checks scattered through commands
   become `may_control` plus adapter-computed standing.
5. **The queue leaves songbird.** This is the step that makes Matrix voice
   possible, and the biggest. It also touches the economy, autoplay and the
   dashboard.
6. **Error split** (4.8).

Steps 1–3 are worth doing even if CrackTunes never leaves Discord. They are the
"business logic detangled from discord specific" the owner asked for, and they
are next. Steps 4–6 pay off only with a second platform. Stoat comes first: it is
closest to Discord, and its voice needs only LiveKit signalling, with no media
encryption to join. Matrix follows.

## 7. Still open

- **Matrix scope.** When a room is in a space, do settings follow the space or the
  room? This design follows the space only when it is configured explicitly.
  Nothing is needed until Matrix is next.
- **The id migration's order.** Is it one migration that rewrites every bigint id
  column, or one per table as each step lands? Step 2 decides it for settings.

## 8. Non-goals

- A working Matrix or Stoat bot.
- Parity for Discord-native features. Voice rooms, the mod log and the pranks are
  Discord by nature and stay Discord-only.
- Abstracting everything. The model covers what a music bot needs: identity,
  place, authority, output, commands, voice. It is not a general chat SDK.

## Sources

- Stoat rename and architecture: [Stoat (software), Wikipedia](https://en.wikipedia.org/wiki/Stoat_(software)), [awesome-stoat](https://github.com/stoatchat/awesome-stoat), [Stoat voice bridge issue (join_call → LiveKit)](https://github.com/satyrnidae/stoat-discord-bridge/issues/113), [Dedimax self-hosting guide (LiveKit, Feb 2026)](https://www.dedimax.com/en/blog/self-host-your-own-discord-the-complete-guide-to-stoat-ex-revolt)
- Guilded's closure: [AlternativeTo, Nov 2025](https://alternativeto.net/news/2025/11/guilded-to-shut-down-on-december-19-2025-as-focus-shifts-to-roblox-communities/)
- Discord DAVE: [Discord blog](https://discord.com/blog/every-voice-and-video-call-on-discord-is-now-end-to-end-encrypted), [Help Net Security, May 2026](https://www.helpnetsecurity.com/2026/05/19/discord-voice-and-video-call-encryption/)
- MatrixRTC and LiveKit: [Element Call](https://github.com/element-hq/element-call), [tomfos.tr on Matrix + LiveKit](https://tomfos.tr/matrix/livekit/)
- Telegram voice bots need a user session: [RadioPlayer](https://github.com/Sadew451/RadioPlayer)
- Fluxer: [sinologic.net review](https://www.sinologic.net/?p=21651)
