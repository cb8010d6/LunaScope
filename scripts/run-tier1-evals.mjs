import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { redactSensitiveText } from "./redact-sensitive.mjs";

const cases = [
  {
    id: "orchestration-graph-validation",
    args: ["test", "-p", "lunascope-core", "--test", "contract"],
  },
  {
    id: "tool-lifecycle",
    args: ["test", "-p", "lunascope-desktop", "provider_tool_lifecycle_is_durable_before_and_after_execution", "--lib"],
  },
  {
    id: "worktree-isolation",
    args: ["test", "-p", "lunascope-runtime", "isolated_workers_handoff_disjoint_patches_to_verifier"],
  },
  {
    id: "verifier-evidence",
    args: ["test", "-p", "lunascope-runtime", "verifier_must_cover_every_acceptance_criterion_with_direct_evidence", "--lib"],
  },
  {
    id: "cancellation",
    args: ["test", "-p", "lunascope-runtime", "cancellation_interrupts_a_running_worker_and_records_terminal_state", "--lib"],
  },
  {
    id: "retry",
    args: ["test", "-p", "lunascope-runtime", "retryable_provider_failure_uses_a_fresh_attempt_and_completes", "--lib"],
  },
  {
    id: "crash-reconciliation",
    args: ["test", "-p", "lunascope-desktop", "restart_reconciles_a_completed_write_without_replaying_the_side_effect", "--lib"],
  },
  {
    id: "ambiguous-side-effect",
    args: ["test", "-p", "lunascope-desktop", "ambiguous_mutating_call_requires_intervention_and_cannot_auto_retry", "--lib"],
  },
  {
    id: "malicious-extension-rejection",
    args: ["test", "-p", "lunascope-extensions", "pinned_quarantine_install_update_and_rollback_never_execute_install_hook", "--lib"],
  },
];

const outputFlag = process.argv.indexOf("--output");
const output = resolve(outputFlag >= 0 ? process.argv[outputFlag + 1] : "artifacts/evals/tier1.json");
const environment = { ...process.env };
const secretValues = Object.entries(environment)
  .filter(([key, value]) => value && /API[_-]?KEY|TOKEN|SECRET|PASSWORD|AUTHORIZATION|(^|_)(KEY|CREDENTIAL|COOKIE)($|_)/i.test(key))
  .map(([, value]) => value);
for (const key of Object.keys(environment)) {
  if (/^GIT_CONFIG(?:_|$)/i.test(key) ||
    /API[_-]?KEY|TOKEN|SECRET|PASSWORD|AUTHORIZATION|(^|_)(KEY|CREDENTIAL|COOKIE)($|_)/i.test(key)) {
    delete environment[key];
  }
}

const evidence = [];
for (const item of cases) {
  const started = Date.now();
  const result = spawnSync("cargo", item.args, {
    cwd: process.cwd(),
    env: environment,
    encoding: "utf8",
    shell: false,
    windowsHide: true,
  });
  const combined = `${result.stdout ?? ""}\n${result.stderr ?? ""}`.trim();
  evidence.push({
    id: item.id,
    command: `cargo ${item.args.join(" ")}`,
    durationMs: Date.now() - started,
    status: result.status === 0 ? "passed" : "failed",
    exitCode: result.status,
    signal: result.signal,
    evidence: redactSensitiveText(combined.slice(-4000), secretValues),
  });
}

const report = {
  schemaVersion: 1,
  tier: "deterministic",
  fakeProviderOnly: true,
  paidCredentialsUsed: false,
  generatedAt: new Date().toISOString(),
  result: evidence.every((item) => item.status === "passed") ? "passed" : "failed",
  cases: evidence,
};
await mkdir(dirname(output), { recursive: true });
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, "utf8");
console.log(`tier1=${report.result} cases=${evidence.length} evidence=${output}`);
if (report.result !== "passed") process.exitCode = 1;
