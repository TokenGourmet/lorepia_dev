# TLS fault-test fixture

`self-signed-cert.der.hex` and `self-signed-key.der.hex` are a deliberately
public, self-signed localhost test pair used only by `network_fault_tests.rs`.
The private-key bytes are not a product credential, are never compiled into a
non-test target, and must not be trusted or reused outside this deterministic
negative TLS test.
