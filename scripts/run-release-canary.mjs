import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { redactSensitiveText } from "./redact-sensitive.mjs";

const secret = process.env.RELEASE_CANARY_DEEPSEEK_API_KEY ?? "";
if (!secret) {
  throw new Error("Release canary credential is unavailable. Configure the GitHub Actions secret and run the manual workflow.");
}

const outputFlag = process.argv.indexOf("--output");
const output = resolve(
  outputFlag >= 0 ? process.argv[outputFlag + 1] : "artifacts/canary/latest.json",
);
const cases = [
  ["provider-reasoning-config", "deepseek_v4_flash_live_worker_canary"],
  ["programming-task", "deepseek_v4_flash_live_file_orchestration_canary"],
  ["research-task", "deepseek_v4_flash_live_research_canary"],
  ["ultranote-task", "deepseek_v4_flash_live_ultranote_attachment_canary"],
  ["multi-agent-task", "deepseek_v4_flash_live_worker_graph_quality_canary"],
  ["browser-verification", "deepseek_v4_flash_live_sorting_visualizer_release_canary"],
];

function redact(value) {
  return redactSensitiveText(value, [secret]);
}

function cargoTest(filter) {
  const environment = { ...process.env };
  for (const key of Object.keys(environment)) {
    if (/^GIT_CONFIG(?:_|$)/i.test(key)) delete environment[key];
  }
  return spawnSync(
    "cargo",
    ["test", "-p", "lunascope-desktop", "--lib", filter, "--", "--ignored", "--nocapture"],
    {
      cwd: process.cwd(),
      env: environment,
      encoding: "utf8",
      shell: false,
      windowsHide: true,
    },
  );
}

const startedAt = new Date();
const evidence = [];
let seeded = false;
try {
  const seed = cargoTest("seed_release_canary_credential_from_environment");
  if (seed.status !== 0) {
    throw new Error(`Could not stage the ephemeral Windows credential: ${redact(seed.stderr).slice(-1200)}`);
  }
  seeded = true;
  for (const [id, filter] of cases) {
    const started = Date.now();
    const result = cargoTest(filter);
    const logs = redact(`${result.stdout ?? ""}\n${result.stderr ?? ""}`).trim();
    evidence.push({
      id,
      provider: "DeepSeek official",
      model: "deepseek-v4-flash",
      durationMs: Date.now() - started,
      result: result.status === 0 ? "passed" : "failed",
      acceptanceCriteriaVerdict: result.status === 0 ? "passed" : "failed",
      failureReason: result.status === 0 ? null : logs.slice(-2000),
      relevantLogs: logs.slice(-4000),
    });
    if (result.status !== 0) break;
  }
} finally {
  if (seeded) {
    const cleanup = cargoTest("remove_release_canary_credential");
    if (cleanup.status !== 0) {
      evidence.push({
        id: "credential-cleanup",
        provider: "DeepSeek official",
        model: "deepseek-v4-flash",
        durationMs: 0,
        result: "failed",
        acceptanceCriteriaVerdict: "failed",
        failureReason: redact(cleanup.stderr).slice(-2000),
        relevantLogs: redact(cleanup.stderr).slice(-4000),
      });
    }
  }
}

const finishedAt = new Date();
const report = {
  schemaVersion: 1,
  tier: "release-canary",
  runId: process.env.GITHUB_RUN_ID ?? `local-${startedAt.getTime()}`,
  commit: process.env.GITHUB_SHA ?? "local-unbound",
  sourceRun: process.env.GITHUB_RUN_ID
    ? `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}`
    : null,
  startedAt: startedAt.toISOString(),
  finishedAt: finishedAt.toISOString(),
  durationMs: finishedAt.getTime() - startedAt.getTime(),
  result:
    evidence.length === cases.length && evidence.every((item) => item.result === "passed")
      ? "passed"
      : "failed",
  credentialStoredInEvidence: false,
  cases: evidence,
};
await mkdir(dirname(output), { recursive: true });
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, "utf8");
console.log(`release_canary=${report.result} cases=${evidence.length}/${cases.length} evidence=${output}`);
if (report.result !== "passed") process.exitCode = 1;
