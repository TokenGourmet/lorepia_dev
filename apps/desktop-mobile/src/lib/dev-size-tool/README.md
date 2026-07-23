# Development window size tool

This overlay exists only in the opt-in Tauri development flavor:

```sh
npm run dev:size-tool
```

Normal `npm run tauri dev` and production builds do not load the overlay or its
window-resize permissions.

## Clean removal

1. Delete this `src/lib/dev-size-tool/` directory.
2. Delete `src-tauri/tauri.dev-size.conf.json`.
3. Remove `dev:size:web` and `dev:size-tool` from `package.json`.
4. Remove the `DevSizeTool` state, guarded dynamic import, and conditional
   component block from `src/routes/+layout.svelte`.
