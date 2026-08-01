# Third-party notices

## Spine Companion

The optional desktop companion module includes selected renderer and state-machine
source files from Spine Companion v0.2.6-rc.10, copyright Spine Companion
contributors, under the MIT License. The retained license text is bundled at
`apps/desktop/src/companion/vendor/LICENSE.spine-companion.txt`.

No character model, texture, or catalog entry from Spine Companion is bundled.
Users must import a local model for which they have the necessary rights.

The model-library JSON files under
`apps/desktop/src-tauri/resources/companion/catalog/` are metadata snapshots from
the pinned Ark-Models repository revision `2f3187f780108847d7327946e1906fc6b80bead3`.
They are not model binaries. Their entries intentionally use `NOASSERTION` where
upstream licensing or redistribution rights have not been verified. Downloads are
performed only after an explicit user action, written to the user's local data
directory, and are never included in source archives or releases.

The Live2D adapter includes the Cubism 4 bridge from `pixi-live2d-display`
0.4.0, copyright its contributors, under the MIT License. Its retained license
is at `apps/desktop/src/companion/vendor/live2d/LICENSE.pixi-live2d-display.txt`.
LunaScope does not bundle Live2D Cubism Core. On the user's first Live2D import,
the app downloads the unmodified Redistributable Code directly from Live2D's
official `cubism.live2d.com` endpoint and stores it in the local companion
runtime directory. Cubism Core remains subject to Live2D's Proprietary Software
License Agreement, linked in that downloaded file's header.

## Document understanding and attachment ingestion

- K-Dense Scientific Agent Skills `markitdown` package, pinned at
  `ab2f84ab10597c59fac186ecda6d5edd5dcc8b92`, MIT License. LunaScope bundles
  the exact upstream `SKILL.md`, references, helper scripts, and license.
  Scripts remain inert unless a separately permissioned tool is explicitly
  added later. Repository: <https://github.com/K-Dense-AI/scientific-agent-skills/tree/ab2f84ab10597c59fac186ecda6d5edd5dcc8b92/skills/markitdown>.
- Microsoft MarkItDown (`microsoft/markitdown`), revision
  `fd239d5d2be43d9b68329730206b9312c7d5a388`, MIT License, is the documented
  upstream conversion design reference. LunaScope does not bundle or execute
  its Python runtime; the shipping extraction path is native Rust.
- `office_oxide` 0.1.8 by Yury Fedoseev and contributors, used for native
  DOC/DOCX, XLS/XLSX, and PPT/PPTX extraction. Dual licensed MIT or
  Apache-2.0; LunaScope elects Apache-2.0 for this dependency.
- `deformat` 0.15.0 by its contributors, used for native PDF text extraction.
  Dual licensed MIT or Apache-2.0; LunaScope elects Apache-2.0.
- `base64` 0.22.1 and `zip` 8.6.0 are used for provider-native multimodal
  payloads and bounded OOXML embedded-image extraction under their published
  permissive licenses.

## HarmonyOS Sans

LunaScope uses **HarmonyOS Sans SC** for all user-interface typography.

Copyright 2021 Huawei Device Co., Ltd.

HarmonyOS Sans Fonts Software is licensed under the **HarmonyOS Sans Fonts License Agreement**. The license permits royalty-free commercial use and allows unmodified font copies to be embedded, bundled, redistributed and/or sold with software other than stand-alone font software, subject to the agreement. Important conditions include:

1. The software must prominently state that it uses HarmonyOS Sans.
2. The font files and their individual components must not be modified.
3. The fonts must not be redistributed or sold on a stand-alone basis.
4. The copyright notice and complete license agreement must remain with every redistributed copy.
5. The fonts are supplied as-is; the complete agreement controls over this summary.

The current repository references locally installed `HarmonyOS Sans SC` faces and does not redistribute font binaries. Before bundling the font in an installer, include only unmodified files from the official package and ship the complete font license agreement beside them.

- Official resources: <https://developer.huawei.com/consumer/en/design/resource/>
- License text: <https://gitee.com/openharmony/global_system_resources/blob/master/LICENSE_Fonts>

## Agent architecture references

LunaScope's runtime prompt, activity projection, and Skill format were independently implemented after reviewing public architecture and documentation. No source code or prompt text from these projects is vendored into LunaScope.

- OpenAI Codex, Apache-2.0: <https://github.com/openai/codex>
- OpenCode, MIT: <https://github.com/anomalyco/opencode>
- Model Context Protocol reference servers, Apache-2.0 / MIT by component: <https://github.com/modelcontextprotocol/servers>
- GitHub MCP Server, MIT: <https://github.com/github/github-mcp-server>

## Bundled system Skills

LunaScope's system Skills are vendored from pinned public repositories rather than written specifically for LunaScope. The complete source mapping and immutable revisions are recorded in `apps/desktop/src-tauri/resources/system-skills/UPSTREAM_SOURCES.md`; each package retains its upstream license text.

- OpenAI Skills, Apache-2.0: <https://github.com/openai/skills>
- Anthropic Skills `frontend-design`, Apache-2.0: <https://github.com/anthropics/skills>
- obra/superpowers, MIT: <https://github.com/obra/superpowers>
- Orchestra Research AI Research Skills, MIT: <https://github.com/Orchestra-Research/AI-research-SKILLs>
- Imbad0202 Academic Research Skills, CC BY-NC 4.0: <https://github.com/Imbad0202/academic-research-skills>
- K-Dense Scientific Agent Skills `markitdown`, MIT: <https://github.com/K-Dense-AI/scientific-agent-skills/tree/ab2f84ab10597c59fac186ecda6d5edd5dcc8b92/skills/markitdown>
- Agents365 Mermaid Skill, revision `150d8d00e7b0b5457a26277d146d5ab5f6fa2e1f`, MIT: <https://github.com/Agents365-ai/mermaid-skill>

## Note and PDF rendering

- Mermaid 11.16.0, MIT License, is bundled as a local minified browser runtime
  for offline mind-map and diagram rendering. The matching license text ships
  beside `mermaid.min.js`; no remote Mermaid service or CDN is used.
- KaTeX 0.18.1, MIT License, is bundled as a local parser plus auto-render
  helper. LunaScope requests MathML output, so formula layout remains local and
  accessible without redistributing the KaTeX font bundle.
- `pulldown-cmark` 0.13.4, MIT License, converts the note-only Markdown artifact
  to semantic HTML before printing.
- Microsoft Edge is invoked from the user's existing Windows installation in
  one-shot headless print mode. Edge is not redistributed by LunaScope.

The Imbad0202 packages are licensed for noncommercial use only. The current
bundle may be used for personal/noncommercial evaluation, but those packages
must be removed or separately licensed before a commercial LunaScope release.
Attribution, license retention, and other upstream terms continue to apply.
