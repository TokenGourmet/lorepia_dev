import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const htmlUrl = new URL("../static/plugin-frame.html", import.meta.url);
const scriptUrl = new URL("../static/plugin-frame.js", import.meta.url);
const scriptOpen = "<script>";
const scriptClose = "</script>";

const source = readFileSync(scriptUrl, "utf8");
if (source.toLowerCase().includes(scriptClose)) {
  throw new Error("plugin-frame.js must not contain a literal </script token");
}

const html = readFileSync(htmlUrl, "utf8");
const start = html.indexOf(scriptOpen);
const end = html.indexOf(scriptClose, start + scriptOpen.length);
if (start < 0 || end < 0 || html.indexOf(scriptClose, end + scriptClose.length) >= 0) {
  throw new Error("plugin-frame.html must contain exactly one inline script");
}

const digest = createHash("sha256").update(source).digest("base64");
const withSource =
  html.slice(0, start + scriptOpen.length) + source + html.slice(end);
const hashPattern = /script-src 'sha256-[A-Za-z0-9+/=]+'/g;
const matches = withSource.match(hashPattern);
if (matches?.length !== 1) {
  throw new Error("plugin-frame.html must contain exactly one script-src SHA-256 hash");
}

writeFileSync(
  htmlUrl,
  withSource.replace(hashPattern, `script-src 'sha256-${digest}'`),
  "utf8",
);
