# transpose

Platform-neutral types for a community music bot, mapped onto Discord, Matrix
and Stoat (formerly Revolt), with Telegram as a capability stress test. It is a
design exercise held apart from cracktunes. Read [`docs/design.md`](docs/design.md) first.

```
crates/core      the neutral types and the bot logic; depends on no chat platform
crates/discord   serenity 0.12 ⇄ core
crates/matrix    ruma 0.17 ⇄ core
crates/stoat     rive-models 1.2 ⇄ core
```

```sh
cargo test --workspace      # 25 tests: the same bot logic run through all three adapters
cargo tree -p transpose-core   # no serenity, ruma or rive in sight
```
