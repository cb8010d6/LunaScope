import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const maximumEvidenceAgeMs = 14 * 24 * 60 * 60 * 1000;
const canaryCases = [
  "programming-task",
  "multi-agent-task",
  "research-task",
  "ultranote-task",
  "provider-reasoning-config",
  "browser-verification",
];
const enduranceCases = [
  "repeated-worker-dispatch-and-retry",
  "cancellation",
  "sqlite-reopen",
  "unclean-wal-recovery",
];
const enduranceCoverage = [
  "repeated Worker dispatch",
  "retries",
  "cancellation",
  "SQLite reopen",
  "unclean WAL recovery",
  "process and system memory sampling",
];

function pushOnce(blockers, value) {
  if (!blockers.includes(value)) blockers.push(value);
}

function hasTrustedProvenance(evidence, repository) {
  if (!(typeof evidence.runId === "string" && /^\d+$/.test(evidence.runId) &&
    typeof evidence.commit === "string" && /^[0-9a-f]{40}$/i.test(evidence.commit) &&
    typeof evidence.sourceRun === "string" &&
    /^https:\/\/github\.com\/[^/]+\/[^/]+\/actions\/runs\/\d+$/i.test(evidence.sourceRun) &&
    evidence.sourceRun.endsWith(`/actions/runs/${evidence.runId}`))) return false;
  return !repository || evidence.sourceRun ===
    `https://github.com/${repository}/actions/runs/${evidence.runId}`;
}

function validEnduranceEvidence(evidence) {
  if (!(evidence.schemaVersion === 1 &&
    evidence.suite === "synthetic-endurance" &&
    Number.isFinite(Date.parse(evidence.startedAt)) &&
    Number.isFinite(Date.parse(evidence.finishedAt)) &&
    Number.isFinite(evidence.durationMs) && evidence.durationMs >= 0 &&
    Number.isFinite(evidence.requestedDurationMinutes) &&
    evidence.requestedDurationMinutes >= 120 &&
    Number.isInteger(evidence.cycles) && evidence.cycles > 0 &&
    Array.isArray(evidence.covers) &&
    enduranceCoverage.every((item) => evidence.covers.includes(item)) &&
    Array.isArray(evidence.samples) && evidence.samples.length === evidence.cycles)) {
    return false;
  }
  return evidence.samples.every((sample, index) => {
    if (!(sample.cycle === index + 1 &&
      Number.isFinite(Date.parse(sample.startedAt)) &&
      Number.isFinite(sample.durationMs) && sample.durationMs >= 0 &&
      Number.isFinite(sample.processRssBytes) && sample.processRssBytes > 0 &&
      Number.isFinite(sample.systemFreeMemoryBytes) && sample.systemFreeMemoryBytes >= 0 &&
      Number.isFinite(sample.systemTotalMemoryBytes) && sample.systemTotalMemoryBytes > 0 &&
      Array.isArray(sample.results))) return false;
    const ids = sample.results.map((item) => item.id);
    if (ids.length !== enduranceCases.length || new Set(ids).size !== ids.length) return false;
    return sample.results.every((item) =>
      enduranceCases.includes(item.id) &&
      item.status === "passed" &&
      item.exitCode === 0 &&
      Number.isFinite(item.durationMs) && item.durationMs >= 0 &&
      typeof item.evidence === "string"
    );
  });
}

function validCanaryEvidence(evidence) {
  if (
    evidence.schemaVersion !== 1 ||
    evidence.tier !== "release-canary" ||
    evidence.credentialStoredInEvidence !== false ||
    !Array.isArray(evidence.cases)
  ) return false;
  const ids = evidence.cases.map((item) => item.id);
  if (new Set(ids).size !== ids.length || ids.length !== canaryCases.length) return false;
  return evidence.cases.every((item) =>
    canaryCases.includes(item.id) &&
    item.result === "passed" &&
    item.acceptanceCriteriaVerdict === "passed" &&
    typeof item.provider === "string" && item.provider.length > 0 &&
    typeof item.model === "string" && item.model.length > 0 &&
    Number.isFinite(item.durationMs) && item.durationMs >= 0 &&
    item.failureReason === null &&
    typeof item.relevantLogs === "string"
  );
}

function containsSensitiveText(value) {
  const text = String(value ?? "");
  const comparable = text
    .replace(/-----BEGIN\s+[A-Z0-9 ]*PRIVATE KEY-----\[REDACTED\]-----END\s+[A-Z0-9 ]*PRIVATE KEY-----/gi, "")
    .replace(/\[REDACTED\]/gi, "")
    .replace(/\bbearer\s*$/i, "");
  return /-----BEGIN\s+[A-Z0-9 ]*PRIVATE KEY-----/i.test(comparable) ||
    /bearer\s+[A-Za-z0-9._~+/-]{12,}/i.test(comparable) ||
    /(?:authorization|api[-_ ]?key|password|secret|access[_ -]?token|refresh[_ -]?token|cookie|credential(?:value|[_ -]?value)?)\s*[:=]\s*["']?[^\s,;}"']+/i.test(comparable) ||
    /(?:^|\s)(?:sk|pk)-[A-Za-z0-9]{16,}(?:$|\s)/i.test(comparable);
}

function containsSensitiveEvidence(evidence) {
  const strings = [];
  const visit = (value) => {
    if (typeof value === "string") {
      strings.push(value);
    } else if (Array.isArray(value)) {
      value.forEach(visit);
    } else if (value && typeof value === "object") {
      Object.values(value).forEach(visit);
    }
  };
  visit(evidence);
  return strings.some(containsSensitiveText);
}

function readEvidence(root, relative, blockerPrefix, kind, now, repository, blockers) {
  const path = resolve(root, relative);
  if (!existsSync(path)) {
    blockers.push(`${blockerPrefix}_EVIDENCE_MISSING`);
    return null;
  }
  try {
    const evidence = JSON.parse(readFileSync(path, "utf8"));
    const finished = Date.parse(evidence.finishedAt ?? evidence.generatedAt ?? "");
    if (evidence.result !== "passed") pushOnce(blockers, `${blockerPrefix}_EVIDENCE_FAILED`);
    if (!Number.isFinite(finished) || finished > now.getTime() || now.getTime() - finished > maximumEvidenceAgeMs) {
      pushOnce(blockers, `${blockerPrefix}_EVIDENCE_STALE`);
    }
    if (!hasTrustedProvenance(evidence, repository)) {
      pushOnce(blockers, `${blockerPrefix}_EVIDENCE_PROVENANCE_INVALID`);
    }
    if (containsSensitiveEvidence(evidence)) {
      pushOnce(blockers, `${blockerPrefix}_EVIDENCE_SENSITIVE_DATA`);
    }
    const shapeValid = kind === "endurance"
      ? validEnduranceEvidence(evidence)
      : validCanaryEvidence(evidence);
    if (!shapeValid) {
      pushOnce(blockers, `${blockerPrefix}_EVIDENCE_SCHEMA_INVALID`);
    }
    return evidence;
  } catch {
    blockers.push(`${blockerPrefix}_EVIDENCE_INVALID`);
    return null;
  }
}

export function evaluateReleaseGate({ root, channel, signingAvailable, repository = null, now = new Date() }) {
  if (!new Set(["rc", "stable"]).has(channel)) {
    throw new Error(`unsupported release channel: ${channel}`);
  }
  const blockers = [];
  if (!existsSync(resolve(root, "docs/THIRD_PARTY_NOTICES.md"))) {
    blockers.push("THIRD_PARTY_NOTICES_MISSING");
  }
  if (channel === "stable" && !existsSync(resolve(root, "LICENSE"))) {
    blockers.push("PROJECT_LICENSE_MISSING");
  }
  if (channel === "stable" && !signingAvailable) {
    blockers.push("WINDOWS_SIGNING_UNAVAILABLE");
  }
  if (channel === "stable") {
    const legalGatePath = resolve(root, "release/legal-gates.json");
    if (!existsSync(legalGatePath)) {
      blockers.push("THIRD_PARTY_LEGAL_GATE_MISSING");
    } else {
      try {
        const legalGate = JSON.parse(readFileSync(legalGatePath, "utf8"));
        const unresolved = legalGate.schemaVersion !== 1 ||
          legalGate.status !== "resolved" ||
          !Array.isArray(legalGate.findings) ||
          legalGate.findings.some((finding) =>
            finding.blocksStable === true &&
            (typeof finding.resolutionEvidence !== "string" || finding.resolutionEvidence.trim() === "")
          );
        if (unresolved) blockers.push("THIRD_PARTY_LEGAL_CONFIRMATION_REQUIRED");
      } catch {
        blockers.push("THIRD_PARTY_LEGAL_GATE_INVALID");
      }
    }
    const endurance = readEvidence(
      root,
      "release/evidence/endurance-latest.json",
      "ENDURANCE",
      "endurance",
      now,
      repository,
      blockers,
    );
    if (endurance && Number(endurance.durationMs) < 2 * 60 * 60 * 1000) {
      blockers.push("ENDURANCE_EVIDENCE_TOO_SHORT");
    }
    const canary = readEvidence(
      root,
      "release/evidence/canary-latest.json",
      "CANARY",
      "canary",
      now,
      repository,
      blockers,
    );
    if (canary) {
      const results = new Map(canary.cases.map((item) => [item.id, item.result]));
      if (canaryCases.some((id) => results.get(id) !== "passed")) blockers.push("CANARY_EVIDENCE_INCOMPLETE");
    }
    if (endurance && canary && endurance.commit !== canary.commit) {
      blockers.push("EVIDENCE_COMMIT_MISMATCH");
    }
  }
  const evidenceCommit = channel === "stable"
    ? (() => {
        try {
          const endurance = JSON.parse(readFileSync(resolve(root, "release/evidence/endurance-latest.json"), "utf8"));
          const canary = JSON.parse(readFileSync(resolve(root, "release/evidence/canary-latest.json"), "utf8"));
          return endurance.commit === canary.commit ? endurance.commit : null;
        } catch {
          return null;
        }
      })()
    : null;
  return {
    channel,
    signingAvailable,
    artifactLabel: signingAvailable ? `${channel}-signed` : "unsigned-rc",
    stableReleaseGateSatisfied: channel === "stable" && blockers.length === 0,
    evidenceCommit,
    allowed: channel === "rc"
      ? !blockers.includes("THIRD_PARTY_NOTICES_MISSING")
      : blockers.length === 0,
    blockers,
  };
}

function valueAfter(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const root = resolve(valueAfter("--root") ?? ".");
  const channel = valueAfter("--channel") ?? "rc";
  const signingAvailable = valueAfter("--signing-available") === "true";
  const repository = valueAfter("--repository") ?? null;
  const result = evaluateReleaseGate({ root, channel, signingAvailable, repository });
  console.log(JSON.stringify(result, null, 2));
  if (!result.allowed) process.exitCode = 1;
}
