export type NativeCommandAuditResult = {
  passed: boolean;
  detail: string;
};

export type NativeInvoke = (
  command: string,
  args?: Record<string, unknown>,
) => Promise<unknown>;

const RETIRED_RAW_COMMANDS: ReadonlyArray<
  readonly [command: string, args: Record<string, unknown> | undefined]
> = [
  ["privileged_probe", undefined],
  ["privileged_probe_count", undefined],
  ["sanitize_plugin_html", { html: "<p>probe</p>" }],
];

class NativeCommandAuditTimeout extends Error {
  constructor(command: string) {
    super(`native command audit timed out for ${command}`);
    this.name = "NativeCommandAuditTimeout";
  }
}

function boundedErrorText(error: unknown): string {
  let text: string;
  if (typeof error === "string") {
    text = error;
  } else if (error instanceof Error) {
    text = error.message;
  } else {
    try {
      text = JSON.stringify(error);
    } catch {
      text = String(error);
    }
  }
  return text.replace(/\s+/g, " ").slice(0, 160);
}

function isRecognizedRuntimeDenial(error: unknown, command: string): boolean {
  const text = boundedErrorText(error);
  if (
    text === `Command ${command} not found` ||
    text === `Command ${command} not allowed by ACL` ||
    text === `${command} not allowed. Command not found`
  ) {
    return true;
  }
  return (
    text.includes(command) &&
    /\bnot allowed on origin\b/i.test(text) &&
    /\bcommand\b[\s\S]{0,128}\bnot found\b/i.test(text)
  );
}

async function invokeWithTimeout(
  invokeNative: NativeInvoke,
  command: string,
  args: Record<string, unknown> | undefined,
  timeoutMs: number,
): Promise<unknown> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      invokeNative(command, args),
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(
          () => reject(new NativeCommandAuditTimeout(command)),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

export async function auditRetiredRawCommands(
  invokeNative: NativeInvoke,
  timeoutMs = 2_000,
): Promise<NativeCommandAuditResult> {
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    throw new Error("native command audit timeout must be a positive integer");
  }

  for (const [command, args] of RETIRED_RAW_COMMANDS) {
    try {
      await invokeWithTimeout(invokeNative, command, args, timeoutMs);
      return {
        passed: false,
        detail: `retired raw command ${command} unexpectedly succeeded`,
      };
    } catch (error) {
      if (error instanceof NativeCommandAuditTimeout) {
        return {
          passed: false,
          detail: `top-frame rejection timed out for ${command}`,
        };
      }
      if (!isRecognizedRuntimeDenial(error, command)) {
        return {
          passed: false,
          detail: `${command} returned an unrecognized denial: ${boundedErrorText(error)}`,
        };
      }
    }
  }

  return {
    passed: true,
    detail: "top frame confirmed all retired raw commands are denied",
  };
}
