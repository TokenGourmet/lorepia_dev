# M-1 stream lifecycle hardening

This draft records the narrow, reversible hardening patch proposed after reviewing the Channel spike.

## Included in this patch

1. Treat `ackTimeoutMs` as a real deadline once the producer is blocked by the bounded in-flight window.
2. Emit one structured `failed` terminal event with code `ACK_TIMEOUT` instead of polling forever.
3. Add a regression test proving that an unacknowledged consumer terminates within the configured bound.
4. Replace the Android `FileProvider` external-storage root mapping with app-scoped export and cache directories.

## Deliberately not included

This patch does not yet define the final production retention policy for a dead WebView. A later change must add a channel-independent completion/recovery path and a bounded request-release policy before M-1 can pass.

## Acceptance criteria

- Existing Rust tests continue to pass.
- Clippy passes with warnings denied.
- A consumer that does not free capacity receives an `ACK_TIMEOUT` terminal failure.
- The stream runner exits rather than polling indefinitely.
- No Android `FileProvider` path exposes the external-storage root.

## Rollback

The work remains isolated on `agent/m1-stream-lifecycle-hardening`. Closing the draft pull request or deleting that branch restores the repository to the untouched `main` state.
