import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const config = JSON.parse(await readFile(
  new URL("../src-tauri/tauri.conf.json", import.meta.url),
  "utf8",
));
const csp = config.app.security.csp;

function directive(name) {
  return csp
    .split(";")
    .map((value) => value.trim())
    .find((value) => value.startsWith(`${name} `)) ?? "";
}

test("Tauri CSP permits only the scoped asset protocol for Companion runtime loads", () => {
  for (const name of ["connect-src", "script-src", "img-src", "media-src"]) {
    const value = directive(name);
    assert.match(value, /\basset:/, `${name} must allow Tauri asset URLs`);
    assert.match(value, /\bhttps?:\/\/asset\.localhost\b/, `${name} must allow the Tauri asset host`);
  }
  assert.deepEqual(config.app.security.assetProtocol.scope, []);
  assert.doesNotMatch(csp, /(?:default-src|script-src|connect-src|img-src|media-src)\s+\*/);
});
