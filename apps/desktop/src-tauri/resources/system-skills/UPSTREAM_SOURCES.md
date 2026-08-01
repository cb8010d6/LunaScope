# Bundled system Skill sources

LunaScope bundles selected, unmodified Skill instructions and references from
the pinned upstream revisions below. Optional conference template bundles are
omitted from the application to keep the release small and avoid redistributing
venue-owned style assets. Skill selection is dynamic: the Orchestration model
receives a bounded catalog of names and descriptions, selects the packages
relevant to the current task, and LunaScope loads only those selected
instructions.

## General-purpose packages

| LunaScope directory | Upstream package | Revision | License |
|---|---|---|---|
| `programming` | `obra/superpowers/skills/systematic-debugging` | `44c9b2d6e889982ac18c27d05a19fefe335194e1` | MIT |
| `verification` | `obra/superpowers/skills/verification-before-completion` | `44c9b2d6e889982ac18c27d05a19fefe335194e1` | MIT |
| `frontend-development` | `anthropics/skills/skills/frontend-design` | `b29e7cf65e5cb78a5ac33d582270551bc74a14eb` | Apache-2.0 |
| `computer-use` | `openai/skills/skills/.curated/playwright` | `49f948faa9258a0c61caceaf225e179651397431` | Apache-2.0 |
| `markitdown` | `K-Dense-AI/scientific-agent-skills/skills/markitdown` | `ab2f84ab10597c59fac186ecda6d5edd5dcc8b92` | MIT |
| `mermaid` | `Agents365-ai/mermaid-skill/skills/mermaid-skill` | `150d8d00e7b0b5457a26277d146d5ab5f6fa2e1f` | MIT |

## Research packages from Orchestra Research

Repository: <https://github.com/Orchestra-Research/AI-research-SKILLs>

Revision: `773a52944ba4747a18bd4ae9ade53fff041adcbc`

License: MIT

| LunaScope directory | Upstream package |
|---|---|
| `research` | `0-autoresearch-skill` |
| `academic-writing` | `20-ml-paper-writing/ml-paper-writing` |
| `research-academic-plotting` | `20-ml-paper-writing/academic-plotting` |
| `research-ideation` | `21-research-ideation/brainstorming-research-ideas` |
| `research-manager` | `22-agent-native-research-artifact/research-manager` |
| `research-rigor-reviewer` | `22-agent-native-research-artifact/rigor-reviewer` |

## Academic research packages from Imbad0202

Repository: <https://github.com/Imbad0202/academic-research-skills>

Revision: `2cf3a51e159458b7a8c8784bb874248e79601f7b`

License: Creative Commons Attribution-NonCommercial 4.0 International

| LunaScope directory | Upstream package |
|---|---|
| `deep-research` | `deep-research` |
| `academic-paper` | `academic-paper` |
| `academic-paper-reviewer` | `academic-paper-reviewer` |
| `academic-pipeline` | `academic-pipeline` |

The upstream directory names and UTF-8 bytes are preserved so sibling and
repository-level references continue to resolve. LunaScope also bundles the
referenced `shared/`, the eight scripts linked directly from the four
`SKILL.md` files, and `.claude/CLAUDE.md`. Upstream test fixtures and unrelated
maintenance scripts are omitted to keep the desktop bundle lightweight. A selected
Worker can read the bundled resources on demand through the read-only
`read_skill_resource` tool. Skill scripts and Claude hooks are never executed
merely because the package was discovered or selected.

Compatibility boundary: LunaScope supports the portable `SKILL.md` workflow,
YAML metadata, package references, templates, examples, schemas, and agent
definition files used by Claude Code and Codex Skills. Provider-specific hook,
slash-command, environment-variable, and plugin-subprocess semantics remain
inert unless a separately permissioned native LunaScope tool implements them.
The vendored `markitdown` package is byte-for-byte upstream instruction,
reference, and helper-script content plus its repository license. LunaScope's
native document extractor does not execute those Python helpers during
discovery, selection, upload, or note generation.

The vendored `mermaid` package retains its upstream `SKILL.md`, five reference
files, and MIT license. LunaScope selects it dynamically when relationships,
hierarchies, or processes are clearer as a diagram. The PDF exporter uses a
separately pinned local Mermaid 11.16.0 browser runtime; no CDN is contacted.

The Imbad0202 packages are noncommercial. They must not be included in a
commercial LunaScope distribution without separate permission from the
copyright holder. Their complete license is bundled in each package as
`UPSTREAM-LICENSE.txt`.
