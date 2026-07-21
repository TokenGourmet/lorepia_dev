import assert from "node:assert/strict";
import test from "node:test";

import { strictCspMarkup } from "./strict-csp-svelte-preprocess.mjs";

const announcer =
  '<div id="svelte-announcer" aria-live="assertive" aria-atomic="true" style="position: absolute; left: 0; top: 0; clip: rect(0 0 0 0); clip-path: inset(50%); overflow: hidden; white-space: nowrap; width: 1px; height: 1px"></div>';

test("moves the generated SvelteKit announcer style into compiled CSS", () => {
  const result = strictCspMarkup({
    content: announcer,
    filename: "/workspace/.svelte-kit/generated/root.svelte",
  });
  assert.doesNotMatch(result.code, /\sstyle=/u);
  assert.match(result.code, /#svelte-announcer \{/u);
  assert.match(result.code, /clip-path: inset\(50%\)/u);
});

test("rejects product-authored inline style attributes and directives", () => {
  for (const content of [
    '<div style="width: 1px"></div>',
    "<div style:width={width}></div>",
  ]) {
    assert.throws(
      () => strictCspMarkup({ content, filename: "/workspace/src/x.svelte" }),
      /STRICT_CSP_INLINE_STYLE/u,
    );
  }
});

test("fails closed when the generated announcer template drifts", () => {
  assert.throws(
    () =>
      strictCspMarkup({
        content: '<div id="svelte-announcer"></div>',
        filename: "/workspace/.svelte-kit/generated/root.svelte",
      }),
    /STRICT_CSP_ANNOUNCER_TEMPLATE_DRIFT/u,
  );
});
