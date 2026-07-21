import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import { strictCspMarkup } from "./scripts/strict-csp-svelte-preprocess.mjs";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: [{ markup: strictCspMarkup }, vitePreprocess()],
  kit: {
    adapter: adapter({
      fallback: "index.html",
    }),
  },
};

export default config;
