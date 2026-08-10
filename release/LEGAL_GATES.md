# External legal release gates

This file records findings only. It does not select LunaScope's project license, grant third-party rights, or replace upstream assets.

## Imbad0202 academic research Skills

- Path: `apps/desktop/src-tauri/resources/system-skills`
- Source: `Imbad0202/academic-research-skills` at `2cf3a51e159458b7a8c8784bb874248e79601f7b`
- Current license: CC BY-NC 4.0
- Finding: the bundled packages are limited to noncommercial use.
- Recommended owner/legal action: obtain separate commercial permission or exclude these exact packages from a commercial distribution. Preserve attribution and the upstream license either way.

## Ark-Models Companion catalog

- Path: `apps/desktop/src-tauri/resources/companion/catalog`
- Source: `isHarryh/Ark-Models` at `2f3187f780108847d7327946e1906fc6b80bead3`
- Current license: `NOASSERTION`
- Finding: LunaScope bundles metadata and local-download workflows, not model binaries, but the upstream use/redistribution terms for the character assets are unverified.
- Recommended owner/legal action: confirm that the intended Stable distribution and download workflow are permitted, or disable unresolved catalog entries in a separately reviewed change. Do not bundle the downloaded assets.

The machine-readable status is in `release/legal-gates.json`. Until each `blocksStable` finding contains reviewed resolution evidence and the status is changed to `resolved`, the Stable workflow remains closed. Unsigned/noncommercial engineering RCs must keep the disclosures in `docs/THIRD_PARTY_NOTICES.md`.
