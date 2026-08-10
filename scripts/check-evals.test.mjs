import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { validateEvalManifest } from "./check-evals.mjs";

const valid = JSON.parse(
  readFileSync(new URL("../evals/manifest.json", import.meta.url), "utf8"),
);

test("the structural manifest stays explicitly not run", () => {
  assert.equal(validateEvalManifest(structuredClone(valid)).executionStatus, "defined_not_run");
});

test("a case cannot smuggle a passed result into the structural manifest", () => {
  const injected = structuredClone(valid);
  injected.cases[0].result = "passed";
  assert.throws(
    () => validateEvalManifest(injected),
    /execution evidence or unknown fields/,
  );
});

test("top-level run evidence cannot masquerade as a manifest", () => {
  const injected = { ...structuredClone(valid), finishedAt: new Date().toISOString() };
  assert.throws(
    () => validateEvalManifest(injected),
    /execution evidence or unknown top-level fields/,
  );
});
