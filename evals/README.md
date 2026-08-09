# LunaScope Evals

`manifest.json` defines the twelve Release Candidate evaluation families required by the engineering contract. They are specifications, not fabricated pass records.

- `executionStatus` remains `defined_not_run` until a future eval runner creates durable run IDs and evidence artifacts for every case.
- `requiredEvidence` is the minimum evidence gate.
- `forbiddenClaims` lists claims that must fail the eval even when the output reads well.
- `npm run evals:check` validates manifest completeness, unique IDs, and the non-fabrication marker. It does not claim that the evals have executed.

Live Domain Pack completion and Release Candidate status remain open until the relevant cases have real provider/tool runs and independently reviewable evidence.
