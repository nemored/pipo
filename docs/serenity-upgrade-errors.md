# Serenity 0.11 -> 0.12 compiler error inventory

This inventory was captured from the first `cargo check` after bumping `serenity` to `0.12`.

## src/discord.rs

| File | Symbol / code area | Error(s) observed | Coverage before migration |
|---|---|---|---|
| `src/discord.rs` | `Shared::{contains_channel,get_channel,get_webhook_id,insert_thread,get_sender,get_thread,set_webhook}` | `E0599` (`as_u64` removed on ID wrappers), key type mismatches after replacing ID APIs | Added direct unit tests for channel/webhook/thread/pin maps (`shared_*` tests) |
| `src/discord.rs` | `RealHandler::{insert_into_messages_table,select_id_from_messages,get_sender_and_thread}` | `E0599`/`E0614` around ID extraction (`as_u64` to `get`) and thread ID handling | Added async tests for `get_sender_and_thread` direct-channel and parent-thread behavior |
| `src/discord.rs` | `EventHandler` impl method signatures (`channel_create`, `channel_update`, `guild_create`, `message_update`, `thread_update`) | `E0195`, `E0050` due to Serenity 0.12 trait signature changes | Covered indirectly by compile-time checks; runtime logic paths covered by new handler-state tests |
| `src/discord.rs` | `guild_create` webhook setup | `E0308` because `create_webhook` now takes `CreateWebhook` builder | Covered by state/webhook mapping tests (logic invariant), plus compile-time API conformance |
| `src/discord.rs` | `parse_content` Discord HTTP calls | `E0308`, `E0277` caused by typed IDs (`GuildId`, `UserId`, `RoleId`, `EmojiId`) in HTTP APIs | Existing parsing logic had no dedicated tests; behavior now guarded by surrounding message-flow tests and compile checks |
| `src/discord.rs` | thread creation helper (`get_threadid`) | `E0599` (`create_public_thread` removed), plus error pattern updates | Covered by existing thread routing invariants in new `get_sender_and_thread_*` tests; API path compile-validated |
| `src/discord.rs` | outbound message edit/send paths (`handle_action_message`, `handle_text_message`, webhook execute/edit/delete) | `E0277`, `E0308`, `E0061`, `E0282` from builder API migration (`EditMessage` / `EditWebhookMessage` / `ExecuteWebhook`) | Covered by existing message-flow codepaths and compile checks; state-related invariants reinforced by new tests |
| `src/discord.rs` | client bootstrap (`connect`) | `E0609` (`cache_and_http` removed from `Client`) | Covered by compile-time check and startup path exercised in existing runtime usage |

## Non-Discord warnings/errors seen during spike

Most remaining diagnostics outside `src/discord.rs` were warnings unrelated to Serenity migration (unused imports/variables, deprecated chrono call, etc.).

