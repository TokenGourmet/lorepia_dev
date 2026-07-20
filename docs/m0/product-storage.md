# Product SQLite storage

This record describes the first product-owned persistence slice. It is an
implementation contract and local test record, not physical-device evidence.

## Ownership and command boundary

The native application opens `lorepia.sqlite3` under Tauri's app-local data
directory. One process owns the database lease; a second app instance fails
closed instead of treating the first process's live request as abandoned.
SQLite runs with foreign keys, WAL mode, and a bounded busy timeout.

The trusted main WebView receives only typed commands for storage status,
creating/listing/deleting chats, loading messages, and reading/updating app
preferences. There is no raw SQL, arbitrary path, migration, credential, or
request-state command.

## Schema v1

Schema v1 stores:

- chats with a character ID, title, timestamps, and optimistic revision;
- ordered user and assistant messages with complete, partial, or failed state;
- native-owned provider request progress and cumulative usage;
- selected provider, per-provider model IDs, theme, and default chat mode;
- an FTS5 trigram index for later local message search.

The schema has no API-key, credential-status, stream control-token, or raw
provider-error column. API keys remain exclusively in the OS credential vault.
Provider error details exposed by a remote body are discarded by the runtime;
storage receives only bounded app-owned failure codes.

Startup checks the exact compiled v1 tables, indexes, triggers, FTS definition,
foreign keys, and schema version before recovery. Unknown future versions and
modified schemas fail closed. Schema evolution requires an explicit reviewed
migration; this slice does not claim a v2 migration yet.

## Streaming durability

Starting a first-chat turn inserts the user message, empty assistant message,
and running request state in one transaction. The bridge checkpoints visible
text/refusal, provider response identity, and cumulative usage at a 4 KiB or
250 ms boundary. Reasoning text, credentials, control tokens, and raw remote
errors are not persisted.

The final checkpoint and complete/cancelled/failed state transition are atomic.
Only after that transaction succeeds may the matching terminal Channel event be
sent. On startup, any remaining running request is marked interrupted with the
stable `APP_RESTARTED` code while its last committed partial response remains
available.

## Current product behavior and limits

The chat route finds or creates the fixed first product chat, restores stored
messages, streams temporary deltas, then reloads canonical rows after a terminal
result. Settings hydrate non-secret preferences and serialize optimistic writes;
model edits are debounced. A normal Tauri close request is held until pending
preference writes finish; browser page-hide and mobile background notifications
remain best-effort signals because an operating-system process kill cannot be
made durable by WebView lifecycle code. Storage unavailability disables sending
instead of silently falling back to volatile history.

Message and chat reads use validated cursors with pages of 200 and 100 rows.
The current first-chat loader scans at most 10,000 chats before refusing to
create a duplicate, and restores at most 10,000 messages or 16 MiB of message
text before failing closed. This is bounded restoration, not an unbounded
chat-history browsing claim.

## Verification boundary

Workspace tests cover schema initialization and tamper rejection, WAL reopen,
lease/concurrency behavior, preferences conflicts, chat persistence and search,
stream checkpoint sequencing, terminal atomicity, and restart recovery. Native
adapter tests cover the closed command surface, non-secret preference DTO, and
stream-to-storage mapping. Frontend tests cover strict response parsing,
preference hydration/writes, chat restoration, and the first-chat surface.

These tests and host compilation do not establish Android/iOS physical-device
runtime behavior or packaged Windows/Linux runtime behavior.
