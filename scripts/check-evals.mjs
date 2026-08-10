import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

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

export function validateEvalManifest(manifest) {
  const topLevelKeys = Object.keys(manifest).sort();
  if (JSON.stringify(topLevelKeys) !== JSON.stringify(["cases", "executionStatus", "schemaVersion"])) {
    throw new Error("eval manifest contains execution evidence or unknown top-level fields");
  }
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
  const allowedCaseKeys = [
    "domain",
    "forbiddenClaims",
    "id",
    "input",
    "requiredEvidence",
    "title",
  ];
  for (const item of manifest.cases) {
    if (JSON.stringify(Object.keys(item).sort()) !== JSON.stringify(allowedCaseKeys)) {
      throw new Error(`eval case contains execution evidence or unknown fields: ${item.id}`);
    }
    if (
      typeof item.title !== "string" ||
      typeof item.domain !== "string" ||
      typeof item.input !== "string" ||
      !Array.isArray(item.requiredEvidence) ||
      item.requiredEvidence.length < 3 ||
      !Array.isArray(item.forbiddenClaims) ||
      item.forbiddenClaims.length < 1
    ) {
      throw new Error(`incomplete eval case: ${item.id}`);
    }
  }
  return manifest;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const manifest = validateEvalManifest(JSON.parse(
    await readFile(new URL("../evals/manifest.json", import.meta.url), "utf8"),
  ));
  console.log(`eval_manifest=valid cases=${manifest.cases.length} status=${manifest.executionStatus}`);
}
