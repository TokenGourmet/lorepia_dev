import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";

import {
  resolveBuildProfile,
  storeSafeAssetGate,
} from "./scripts/build-profile.mjs";

const host = process.env.TAURI_DEV_HOST;
const buildProfile = resolveBuildProfile(process.env.TAURI_ENV_PLATFORM);

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [storeSafeAssetGate(buildProfile), sveltekit()],
  define: {
    __LOREPIA_BUILD_PROFILE__: JSON.stringify(buildProfile),
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
