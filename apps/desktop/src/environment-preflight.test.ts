import assert from "node:assert/strict";
import test from "node:test";

import { requiredCapabilitiesForPlan } from "./environment-preflight.ts";

function plan(worker: Partial<{
  role: string;
  tags: string[];
  task: string;
  prompt: string;
  tools: string[];
  completionCriteria: string[];
}> = {}) {
  return {
    objective: "Update the requested files",
    workers: [{
      role: "documentation",
      tags: ["writing"],
      task: "Edit Markdown documentation",
      prompt: "Use local sources only",
      tools: ["filesystem.read", "filesystem.patch"],
      completionCriteria: ["The document is clear"],
      ...worker,
    }],
  };
}

test("a Markdown task is not blocked by an unrelated Cargo manifest", () => {
  assert.deepEqual(requiredCapabilitiesForPlan(plan(), ["Cargo.toml"]), []);
});

test("a Rust implementation with process verification requires Cargo", () => {
  assert.deepEqual(
    requiredCapabilitiesForPlan(
      plan({
        role: "backend",
        task: "Implement and test the Rust runtime",
        tools: ["filesystem.patch", "process.run"],
      }),
      ["Cargo.toml"],
    ),
    ["cargo"],
  );
});

test("frontend browser work requests only its real runtime dependencies", () => {
  assert.deepEqual(
    requiredCapabilitiesForPlan(
      plan({
        role: "frontend",
        task: "Implement and verify the TypeScript interface",
        tools: ["filesystem.patch", "process.run", "check_browser_page"],
      }),
      ["package.json", "package-lock.json", "Cargo.toml"],
    ),
    ["browser", "node", "npm"],
  );
});

test("Python capability is required only for an executable Python plan", () => {
  assert.deepEqual(
    requiredCapabilitiesForPlan(
      plan({
        role: "builder",
        task: "Implement Python and run pytest",
        tools: ["filesystem.patch", "process.run"],
      }),
      ["pyproject.toml"],
    ),
    ["python"],
  );
});
