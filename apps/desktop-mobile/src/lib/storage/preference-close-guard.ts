export type CloseRequestEvent = Readonly<{
  preventDefault(): void;
}>;

type CloseRequestHandler = (event: CloseRequestEvent) => Promise<void>;

export function createPreferenceCloseHandler(
  flush: () => Promise<void>,
  destroy: () => Promise<void>,
): CloseRequestHandler {
  let closing = false;
  return async (event) => {
    event.preventDefault();
    if (closing) return;
    closing = true;
    try {
      await flush();
    } finally {
      await destroy();
    }
  };
}
