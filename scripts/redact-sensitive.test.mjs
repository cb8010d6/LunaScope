import assert from "node:assert/strict";
import test from "node:test";

import { redactSensitiveText } from "./redact-sensitive.mjs";

test("redacts credential-shaped logs before evidence is persisted", () => {
  const input = "Authorization\t:\t'Bearer leaked-token' x-api-key = `sk-live-1234567890123456` cookie=session-value";
  const redacted = redactSensitiveText(input, ["secret-from-environment"]);
  assert.ok(!redacted.includes("leaked-token"));
  assert.ok(!redacted.includes("sk-live-1234567890123456"));
  assert.ok(!redacted.includes("session-value"));
});

test("redacts private keys and explicit secret values", () => {
  const redacted = redactSensitiveText(
    "value=secret-from-environment\n-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----",
    ["secret-from-environment"],
  );
  assert.ok(!redacted.includes("secret-from-environment"));
  assert.ok(!redacted.includes("BEGIN PRIVATE KEY"));
  assert.ok(redacted.includes("[PRIVATE KEY REDACTED]"));
});
