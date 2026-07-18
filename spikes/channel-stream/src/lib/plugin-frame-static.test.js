import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

const htmlPath = new URL("../../static/plugin-frame.html", import.meta.url);
const scriptPath = new URL("../../static/plugin-frame.js", import.meta.url);

describe("packaged plugin frame bootstrap", () => {
  it("keeps the audited inline fixture and CSP hash synchronized", () => {
    const html = readFileSync(htmlPath, "utf8");
    const source = readFileSync(scriptPath, "utf8");
    const start = html.indexOf("<script>") + "<script>".length;
    const end = html.indexOf("</script>", start);

    expect(start).toBeGreaterThan("<script>".length - 1);
    expect(end).toBeGreaterThan(start);
    expect(html.indexOf("</script>", end + 1)).toBe(-1);

    const inline = html.slice(start, end);
    expect(inline).toBe(source);
    expect(inline).not.toContain("</script");

    const digest = createHash("sha256").update(inline).digest("base64");
    expect(html).toContain(`script-src 'sha256-${digest}'`);
  });
});
