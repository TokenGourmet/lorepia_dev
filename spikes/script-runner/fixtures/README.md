# Script Runner security fixtures

These seven JavaScript sources are self-authored Apache-2.0 fixtures. They are
not imported user content and do not define the future public card API.

`catalog.json` pins the exact UTF-8 byte count and SHA-256 of every source. The
spike bundles the sources as trusted test inputs so no source, input JSON, file
path, limit, or engine option crosses Tauri IPC.

The suite adds a separate raw Worker busy-loop case in trusted harness code to
prove that the host-side watchdog can terminate a wedged execution thread even
if the engine-level interrupt path fails.
