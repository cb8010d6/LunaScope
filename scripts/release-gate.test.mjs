import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { evaluateReleaseGate } from "./release-gate.mjs";

function fixture({ license = false, evidence = true } = {}) {
  const root = mkdtempSync(join(tmpdir(), "lunascope-release-gate-"));
  mkdirSync(join(root, "docs"));
  writeFileSync(join(root, "docs", "THIRD_PARTY_NOTICES.md"), "notices\n");
  mkdirSync(join(root, "release"), { recursive: true });
  writeFileSync(
    join(root, "release", "legal-gates.json"),
    JSON.stringify({ schemaVersion: 1, status: "resolved", findings: [] }),
  );
  if (license) writeFileSync(join(root, "LICENSE"), "owner-approved license\n");
  if (evidence) {
    mkdirSync(join(root, "release", "evidence"), { recursive: true });
    writeFileSync(
      join(root, "release", "evidence", "endurance-latest.json"),
      JSON.stringify({
        schemaVersion: 1,
        suite: "synthetic-endurance",
        runId: "1001",
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        sourceRun: "https://github.com/LagrangeNSS/LunaScope/actions/runs/1001",
        result: "passed",
        startedAt: "2026-08-08T21:00:00Z",
        finishedAt: "2026-08-09T00:00:00Z",
        durationMs: 3 * 60 * 60 * 1000,
        requestedDurationMinutes: 180,
        cycles: 1,
        covers: [
          "repeated Worker dispatch",
          "retries",
          "cancellation",
          "SQLite reopen",
          "unclean WAL recovery",
          "process and system memory sampling",
        ],
        samples: [{
          cycle: 1,
          startedAt: "2026-08-08T21:00:00Z",
          durationMs: 3 * 60 * 60 * 1000,
          processRssBytes: 64 * 1024 * 1024,
          systemFreeMemoryBytes: 4 * 1024 * 1024 * 1024,
          systemTotalMemoryBytes: 16 * 1024 * 1024 * 1024,
          results: [
            "repeated-worker-dispatch-and-retry",
            "cancellation",
            "sqlite-reopen",
            "unclean-wal-recovery",
          ].map((id) => ({
            id,
            status: "passed",
            exitCode: 0,
            durationMs: 1,
            evidence: "redacted fixture",
          })),
        }],
      }),
    );
    writeFileSync(
      join(root, "release", "evidence", "canary-latest.json"),
      JSON.stringify({
        schemaVersion: 1,
        tier: "release-canary",
        runId: "1002",
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        sourceRun: "https://github.com/LagrangeNSS/LunaScope/actions/runs/1002",
        result: "passed",
        finishedAt: "2026-08-09T00:00:00Z",
        credentialStoredInEvidence: false,
        cases: [
          "programming-task",
          "multi-agent-task",
          "research-task",
          "ultranote-task",
          "provider-reasoning-config",
          "browser-verification",
        ].map((id) => ({
          id,
          provider: "fixture-provider",
          model: "fixture-model",
          durationMs: 1,
          result: "passed",
          acceptanceCriteriaVerdict: "passed",
          failureReason: null,
          relevantLogs: "redacted fixture",
        })),
      }),
    );
  }
  return root;
}

const now = new Date("2026-08-10T00:00:00Z");

test("unsigned RC is explicit and allowed", () => {
  const result = evaluateReleaseGate({
    root: fixture(),
    channel: "rc",
    signingAvailable: false,
    now,
  });
  assert.equal(result.allowed, true);
  assert.equal(result.artifactLabel, "unsigned-rc");
  assert.equal(result.stableReleaseGateSatisfied, false);
});

test("stable release fails without the owner-selected root license", () => {
  const result = evaluateReleaseGate({
    root: fixture(),
    channel: "stable",
    signingAvailable: true,
    now,
  });
  assert.equal(result.allowed, false);
  assert.deepEqual(result.blockers, ["PROJECT_LICENSE_MISSING"]);
});

test("stable release requires both license and authentic signing material", () => {
  const root = fixture({ license: true });
  assert.equal(
    evaluateReleaseGate({ root, channel: "stable", signingAvailable: false, now })
      .allowed,
    false,
  );
  assert.equal(
    evaluateReleaseGate({ root, channel: "stable", signingAvailable: true, now })
      .stableReleaseGateSatisfied,
    true,
  );
});

test("stable release requires recent canary and multi-hour endurance evidence", () => {
  const result = evaluateReleaseGate({
    root: fixture({ license: true, evidence: false }),
    channel: "stable",
    signingAvailable: true,
    now,
  });
  assert.equal(result.allowed, false);
  assert.deepEqual(result.blockers, [
    "ENDURANCE_EVIDENCE_MISSING",
    "CANARY_EVIDENCE_MISSING",
  ]);
});

test("hand-written pass markers without trusted provenance do not satisfy Stable", () => {
  const root = fixture({ license: true });
  writeFileSync(
    join(root, "release", "evidence", "canary-latest.json"),
    JSON.stringify({ result: "passed", finishedAt: "2026-08-09T00:00:00Z", cases: [] }),
  );
  const result = evaluateReleaseGate({
    root,
    channel: "stable",
    signingAvailable: true,
    now,
  });
  assert.equal(result.allowed, false);
  assert.ok(result.blockers.includes("CANARY_EVIDENCE_PROVENANCE_INVALID"));
  assert.ok(result.blockers.includes("CANARY_EVIDENCE_SCHEMA_INVALID"));
});

test("endurance evidence must contain every deterministic reliability result", () => {
  const root = fixture({ license: true });
  const endurancePath = join(root, "release", "evidence", "endurance-latest.json");
  const endurance = JSON.parse(readFileSync(endurancePath, "utf8"));
  endurance.samples[0].results = [];
  writeFileSync(endurancePath, JSON.stringify(endurance));
  const result = evaluateReleaseGate({ root, channel: "stable", signingAvailable: true, now });
  assert.equal(result.allowed, false);
  assert.ok(result.blockers.includes("ENDURANCE_EVIDENCE_SCHEMA_INVALID"));
});

test("Stable evidence rejects credentials embedded in captured logs", () => {
  const root = fixture({ license: true });
  const canaryPath = join(root, "release", "evidence", "canary-latest.json");
  const canary = JSON.parse(readFileSync(canaryPath, "utf8"));
  canary.cases[0].relevantLogs = "Authorization: Bearer leaked-provider-token";
  writeFileSync(canaryPath, JSON.stringify(canary));
  const result = evaluateReleaseGate({ root, channel: "stable", signingAvailable: true, now });
  assert.equal(result.allowed, false);
  assert.ok(result.blockers.includes("CANARY_EVIDENCE_SENSITIVE_DATA"));
});

test("Stable evidence accepts logs whose sensitive values are explicitly redacted", () => {
  const root = fixture({ license: true });
  const canaryPath = join(root, "release", "evidence", "canary-latest.json");
  const canary = JSON.parse(readFileSync(canaryPath, "utf8"));
  canary.cases[0].relevantLogs = "Authorization: Bearer [REDACTED]";
  writeFileSync(canaryPath, JSON.stringify(canary));
  const result = evaluateReleaseGate({ root, channel: "stable", signingAvailable: true, now });
  assert.equal(result.allowed, true);
});

test("Stable evidence provenance is bound to the release repository", () => {
  const result = evaluateReleaseGate({
    root: fixture({ license: true }),
    channel: "stable",
    signingAvailable: true,
    repository: "another-owner/LunaScope",
    now,
  });
  assert.equal(result.allowed, false);
  assert.ok(result.blockers.includes("ENDURANCE_EVIDENCE_PROVENANCE_INVALID"));
  assert.ok(result.blockers.includes("CANARY_EVIDENCE_PROVENANCE_INVALID"));
});

test("canary and endurance evidence must describe the same source commit", () => {
  const root = fixture({ license: true });
  const canaryPath = join(root, "release", "evidence", "canary-latest.json");
  const canary = JSON.parse(readFileSync(canaryPath, "utf8"));
  canary.commit = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  writeFileSync(canaryPath, JSON.stringify(canary));
  const result = evaluateReleaseGate({ root, channel: "stable", signingAvailable: true, now });
  assert.equal(result.allowed, false);
  assert.ok(result.blockers.includes("EVIDENCE_COMMIT_MISMATCH"));
});

test("Stable remains closed while third-party distribution findings are unresolved", () => {
  const root = fixture({ license: true });
  writeFileSync(
    join(root, "release", "legal-gates.json"),
    JSON.stringify({
      schemaVersion: 1,
      status: "requires_owner_or_legal_review",
      findings: [{ blocksStable: true, resolutionEvidence: null }],
    }),
  );
  const result = evaluateReleaseGate({ root, channel: "stable", signingAvailable: true, now });
  assert.equal(result.allowed, false);
  assert.ok(result.blockers.includes("THIRD_PARTY_LEGAL_CONFIRMATION_REQUIRED"));
});
