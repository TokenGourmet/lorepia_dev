(_input) => ({
  tauri: typeof globalThis.__TAURI__ === "undefined",
  tauriInternals: typeof globalThis.__TAURI_INTERNALS__ === "undefined",
  window: typeof globalThis.window === "undefined",
  document: typeof globalThis.document === "undefined",
  fetch: typeof globalThis.fetch === "undefined",
  xhr: typeof globalThis.XMLHttpRequest === "undefined",
  webSocket: typeof globalThis.WebSocket === "undefined",
  worker: typeof globalThis.Worker === "undefined",
  importScripts: typeof globalThis.importScripts === "undefined",
  process: typeof globalThis.process === "undefined",
  require: typeof globalThis.require === "undefined",
  deno: typeof globalThis.Deno === "undefined",
})
