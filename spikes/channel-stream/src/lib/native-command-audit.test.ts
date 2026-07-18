import { describe, expect, it, vi } from "vitest";

import { auditRetiredRawCommands, type NativeInvoke } from "./native-command-audit";

describe("retired native command audit", () => {
  it("passes only after every retired command returns command-not-found", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async (command) => {
      throw `Command ${command} not found`;
    });

    await expect(auditRetiredRawCommands(invokeNative)).resolves.toEqual({
      passed: true,
      detail: "top frame confirmed all retired raw commands are denied",
    });
    expect(invokeNative.mock.calls.map(([command]) => command)).toEqual([
      "privileged_probe",
      "privileged_probe_count",
      "sanitize_plugin_html",
    ]);
  });

  it("accepts the Tauri ACL form only when it names the retired command", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async (command) => {
      throw `${command} not allowed on origin [local]. Command not found`;
    });

    await expect(auditRetiredRawCommands(invokeNative)).resolves.toMatchObject({
      passed: true,
    });
  });

  it("accepts the exact release ACL denial when binary attestation is separate", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async (command) => {
      throw `Command ${command} not allowed by ACL`;
    });

    await expect(auditRetiredRawCommands(invokeNative)).resolves.toMatchObject({
      passed: true,
    });
  });

  it("accepts the exact Android Runtime Authority denial", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async (command) => {
      throw `${command} not allowed. Command not found`;
    });

    await expect(auditRetiredRawCommands(invokeNative)).resolves.toMatchObject({
      passed: true,
    });
  });

  it("fails closed when a retired command resolves", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => ({ ok: true }));

    await expect(auditRetiredRawCommands(invokeNative)).resolves.toEqual({
      passed: false,
      detail: "retired raw command privileged_probe unexpectedly succeeded",
    });
  });

  it("does not mistake an argument or permission error for command removal", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => {
      throw "invalid args: missing html";
    });

    await expect(auditRetiredRawCommands(invokeNative)).resolves.toEqual({
      passed: false,
      detail:
        "privileged_probe returned an unrecognized denial: invalid args: missing html",
    });
  });

  it("fails closed when the trusted top-frame callback times out", async () => {
    const invokeNative = vi.fn<NativeInvoke>(() => new Promise(() => undefined));

    await expect(auditRetiredRawCommands(invokeNative, 1)).resolves.toEqual({
      passed: false,
      detail: "top-frame rejection timed out for privileged_probe",
    });
  });

  it("rejects invalid timeout configuration", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => undefined);

    await expect(auditRetiredRawCommands(invokeNative, 0)).rejects.toThrow(
      "native command audit timeout must be a positive integer",
    );
    expect(invokeNative).not.toHaveBeenCalled();
  });
});
