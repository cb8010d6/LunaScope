# Release evidence gate

Stable releases require two recent, sanitized evidence files in this directory:

- `endurance-latest.json`: a successful `npm run endurance` result lasting at
  least two hours.
- `canary-latest.json`: successful secret-enabled release canaries covering a
  programming task, multi-Agent task, research task, UltraNote task, Provider
  reasoning compatibility, and browser verification.

Both files must be no older than 14 days. They may contain run IDs, Provider and
model names, durations, verdicts, redacted failure reasons, and links to CI
artifacts. They must never contain API keys, Authorization headers, credential
values, cookies, signing certificates, or other secrets.

Evidence must be produced by the manual GitHub Actions workflows on the same
source commit. Copy the reviewed `artifacts/endurance/latest.json` and
`artifacts/canary/latest.json` outputs to the names above in a release-only
change. The gate validates schema, suite/tier, GitHub run ID and repository
provenance, exact canary coverage, every deterministic endurance result and
memory sample, duration, matching 40-character commit IDs, and age; the Release
workflow also proves that evidence commit is an ancestor of the release tag.

The files are deliberately absent until those suites actually pass. Do not add
placeholder `passed` evidence. RC artifacts can still be produced as
`unsigned-rc`; the Stable gate remains closed.
