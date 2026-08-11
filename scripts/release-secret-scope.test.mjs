import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const releaseWorkflow = readFileSync(".github/workflows/release.yml", "utf8");
const canaryWorkflow = readFileSync(".github/workflows/release-canary.yml", "utf8");
const canaryScript = readFileSync("scripts/run-release-canary.mjs", "utf8");

test("Windows signing secrets are limited to signing steps", () => {
  assert.match(releaseWorkflow, /build-windows:[\s\S]*?environment:\r?\n\s+name: release-signing/);
  assert.doesNotMatch(
    releaseWorkflow,
    /build-windows:[\s\S]*?\n\s+env:\s\n\s+WINDOWS_SIGNING_CERTIFICATE:/,
  );
  assert.equal(
    (releaseWorkflow.match(/WINDOWS_SIGNING_CERTIFICATE:/g) ?? []).length,
    2,
  );
  assert.equal(
    (releaseWorkflow.match(/WINDOWS_SIGNING_CERTIFICATE_PASSWORD:/g) ?? []).length,
    2,
  );
});

test("DeepSeek key is limited to the paid canary and stripped from unrelated Cargo tests", () => {
  assert.match(canaryWorkflow, /live-canary:[\s\S]*?environment:\r?\n\s+name: release-canary/);
  assert.doesNotMatch(
    canaryWorkflow,
    /live-canary:[\s\S]*?\n\s+env:\s\n\s+RELEASE_CANARY_DEEPSEEK_API_KEY:/,
  );
  assert.match(
    canaryWorkflow,
    /name: Run representative live Provider canaries[\s\S]*?RELEASE_CANARY_DEEPSEEK_API_KEY:/,
  );
  assert.match(canaryScript, /delete environment\.RELEASE_CANARY_DEEPSEEK_API_KEY/);
  assert.match(canaryScript, /includeCanarySecret: true/);
});
