# Lua-limits M-1 fixtures

These fixed Lua sources were self-authored for the LorePia M-1 execution-limit
probe. They contain no copied upstream implementation or user content and are
offered under `CC0-1.0` for test-fixture reuse.

The native probe embeds every source with `include_str!`; it does not read Lua
source from a runtime path. [`catalog.json`](catalog.json) pins the policy,
order, UTF-8 byte size, and SHA-256 of every source. The catalog itself is 1,376
bytes with SHA-256
`9ea567d6901ec39412e73f439ee9ea7d47538baea4d1a92cd409c9f3e9b97db5`.

| Fixture | Bytes | SHA-256 | Purpose |
|---|---:|---|---|
| `allowed.lua` | 78 | `a77c3e6430bd50fb97b19b1389679757496fbb77122c99a447bd1e02955d8846` | Deterministic allowed result `55` and recovery sentinel |
| `infinite-loop.lua` | 18 | `ca4214690b93596df8df780783dabb39c0ae2709189d89a02276e39539f2ec1b` | Instruction/deadline interruption |
| `recursive-pressure.lua` | 69 | `a112019c8418f708e82c620c252cab1fffa436bda7f0c57d5d5601110dcc7bbe` | Bounded recursive-pressure interruption |
| `allocator-pressure.lua` | 112 | `91b6256628e808f00cb73138410d33b7268af39c90835fc3d3ca210b52c561c8` | Absolute allocator-ceiling interruption |
| `forbidden-globals.lua` | 281 | `e1fa1c6ef964c4d4d7f20da8c77ebefed8e36088488d32a0d84f2979eb7e2933` | Dangerous and bypass global absence |
| `bypass-surfaces.lua` | 121 | `9325642a6b6a9b439a3a530a993b92e1bf8ba9a5417a890c0fdbfd02b0f0a0ae` | `pcall`/`xpcall`/coroutine bypass-surface absence |
