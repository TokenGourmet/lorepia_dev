const ANNOUNCER_STYLE =
  ' style="position: absolute; left: 0; top: 0; clip: rect(0 0 0 0); clip-path: inset(50%); overflow: hidden; white-space: nowrap; width: 1px; height: 1px"';

const ANNOUNCER_CSS = `
<style>
  #svelte-announcer {
    position: absolute;
    left: 0;
    top: 0;
    clip: rect(0 0 0 0);
    clip-path: inset(50%);
    overflow: hidden;
    white-space: nowrap;
    width: 1px;
    height: 1px;
  }
</style>
`;

function isGeneratedRoot(filename) {
  return filename
    ?.replaceAll("\\", "/")
    .endsWith("/.svelte-kit/generated/root.svelte");
}

function assertNoInlineElementStyle(code, filename) {
  if (/<[^>]+\sstyle(?::|=)/u.test(code)) {
    throw new Error(`STRICT_CSP_INLINE_STYLE:${filename ?? "unknown"}`);
  }
}

export function strictCspMarkup({ content, filename }) {
  let code = content;
  if (isGeneratedRoot(filename)) {
    const occurrences = code.split(ANNOUNCER_STYLE).length - 1;
    if (occurrences !== 1) {
      throw new Error(`STRICT_CSP_ANNOUNCER_TEMPLATE_DRIFT:${occurrences}`);
    }
    code = `${code.replace(ANNOUNCER_STYLE, "")}${ANNOUNCER_CSS}`;
  }
  assertNoInlineElementStyle(code, filename);
  return { code };
}
