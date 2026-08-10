import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { freemem, totalmem } from "node:os";
import { dirname, resolve } from "node:path";
import { redactSensitiveText } from "./redact-sensitive.mjs";

function option(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : fallback;
}

const durationMinutes = Number(option("--duration-minutes", "180"));
const maximumCycles = Number(option("--cycles", "0"));
const output = resolve(option("--output", "artifacts/endurance/latest.json"));
if (!Number.isFinite(durationMinutes) || durationMinutes < 0) {
  throw new Error("duration minutes must be a non-negative number");
}
if (!Number.isInteger(maximumCycles) || maximumCycles < 0) {
  throw new Error("cycles must be a non-negative integer");
}

const commands = [
  ["repeated-worker-dispatch-and-retry", ["test", "-p", "lunascope-runtime", "retryable_provider_failure_uses_a_fresh_attempt_and_completes", "--lib"]],
  ["cancellation", ["test", "-p", "lunascope-runtime", "cancellation_interrupts_a_running_worker_and_records_terminal_state", "--lib"]],
  ["sqlite-reopen", ["test", "-p", "lunascope-storage", "--test", "event_store", "restart_recovers_from_snapshot_plus_committed_events"]],
  ["unclean-wal-recovery", ["test", "-p", "lunascope-storage", "--test", "reliability", "unclean_process_exit_recovers_committed_wal"]],
];
const environment = { ...process.env };
const secretValues = Object.entries(environment)
  .filter(([key, value]) => value && /API[_-]?KEY|TOKEN|SECRET|PASSWORD|AUTHORIZATION|(^|_)(KEY|CREDENTIAL|COOKIE)($|_)/i.test(key))
  .map(([, value]) => value);
for (const key of Object.keys(environment)) {
  if (/^GIT_CONFIG(?:_|$)/i.test(key) ||
    /API[_-]?KEY|TOKEN|SECRET|PASSWORD|AUTHORIZATION|(^|_)(KEY|CREDENTIAL|COOKIE)($|_)/i.test(key)) delete environment[key];
}

const startedAt = new Date();
const deadline = startedAt.getTime() + durationMinutes * 60_000;
const cycles = [];
let cycleIndex = 0;
let failed = false;
while (!failed && (Date.now() < deadline || cycleIndex === 0)) {
  if (maximumCycles > 0 && cycleIndex >= maximumCycles) break;
  cycleIndex += 1;
  const cycleStarted = Date.now();
  const results = [];
  for (const [id, args] of commands) {
    const commandStarted = Date.now();
    const result = spawnSync("cargo", args, {
      cwd: process.cwd(),
      env: environment,
      encoding: "utf8",
      shell: false,
      windowsHide: true,
    });
    const status = result.status === 0 ? "passed" : "failed";
    results.push({
      id,
      status,
      exitCode: result.status,
      durationMs: Date.now() - commandStarted,
      evidence: redactSensitiveText(
        `${result.stdout ?? ""}\n${result.stderr ?? ""}`.trim(),
        secretValues,
      ).slice(-2000),
    });
    if (status === "failed") {
      failed = true;
      break;
    }
  }
  cycles.push({
    cycle: cycleIndex,
    startedAt: new Date(cycleStarted).toISOString(),
    durationMs: Date.now() - cycleStarted,
    processRssBytes: process.memoryUsage().rss,
    systemFreeMemoryBytes: freemem(),
    systemTotalMemoryBytes: totalmem(),
    results,
  });
}

const finishedAt = new Date();
const report = {
  schemaVersion: 1,
  suite: "synthetic-endurance",
  runId: process.env.GITHUB_RUN_ID ?? `local-${startedAt.getTime()}`,
  commit: process.env.GITHUB_SHA ?? "local-unbound",
  sourceRun: process.env.GITHUB_RUN_ID
    ? `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}`
    : null,
  startedAt: startedAt.toISOString(),
  finishedAt: finishedAt.toISOString(),
  durationMs: finishedAt.getTime() - startedAt.getTime(),
  requestedDurationMinutes: durationMinutes,
  cycles: cycles.length,
  result: failed ? "failed" : "passed",
  covers: [
    "repeated Worker dispatch",
    "retries",
    "cancellation",
    "SQLite reopen",
    "unclean WAL recovery",
    "process and system memory sampling",
  ],
  samples: cycles,
};
await mkdir(dirname(output), { recursive: true });
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, "utf8");
console.log(`endurance=${report.result} cycles=${report.cycles} duration_ms=${report.durationMs} evidence=${output}`);
if (failed) process.exitCode = 1;
