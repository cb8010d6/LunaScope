import { readFile } from "node:fs/promises";

const requiredIds = [
  "rust-bug-fix",
  "responsive-frontend-change",
  "source-grounded-research",
  "academic-writing-citation-check",
  "unity-game-design-analysis",
  "github-skill-import",
  "malicious-extension-import",
  "multi-agent-build-review",
  "syllabus-prestudy-plan",
  "lecture-notes",
  "retrieval-review",
  "graded-homework-policy",
];

const manifest = JSON.parse(
  await readFile(new URL("../evals/manifest.json", import.meta.url), "utf8"),
);

if (manifest.schemaVersion !== 1) {
  throw new Error("eval manifest schemaVersion must be 1");
}
if (manifest.executionStatus !== "defined_not_run") {
  throw new Error("structural eval manifest must not claim execution success");
}
if (!Array.isArray(manifest.cases) || manifest.cases.length !== requiredIds.length) {
  throw new Error(`eval manifest must contain ${requiredIds.length} cases`);
}

const ids = manifest.cases.map((item) => item.id);
if (new Set(ids).size !== ids.length) {
  throw new Error("eval case IDs must be unique");
}
for (const id of requiredIds) {
  if (!ids.includes(id)) throw new Error(`missing eval case: ${id}`);
}
for (const item of manifest.cases) {
  if (
    typeof item.title !== "string" ||
    typeof item.input !== "string" ||
    !Array.isArray(item.requiredEvidence) ||
    item.requiredEvidence.length < 3 ||
    !Array.isArray(item.forbiddenClaims) ||
    item.forbiddenClaims.length < 1
  ) {
    throw new Error(`incomplete eval case: ${item.id}`);
  }
}

console.log(`eval_manifest=valid cases=${manifest.cases.length} status=${manifest.executionStatus}`);
