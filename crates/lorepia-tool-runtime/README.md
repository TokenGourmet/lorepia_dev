# lorepia-tool-runtime

This crate defines LorePia's UI-independent safety boundary for model-requested
tools. It provides validated tool definitions and calls, a deny-by-default
allowlist policy, call-bound one-time approval tokens, and a registry for
explicitly supplied application executors.

It deliberately does **not** contain:

- a shell, process, native-command, or filesystem executor;
- an MCP network transport or provider request loop;
- secret values in remote MCP configuration;
- a policy mode that skips per-call approval.

`RemoteMcpServerConfig` is configuration validation only. A future transport
must additionally resolve DNS and reject non-public results immediately before
every connection; this crate cannot make that runtime guarantee without doing
network I/O.
