import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import type { CompanionRenderer } from "./renderer";
import type { CompanionSettings } from "./types";

type Translate = (chinese: string, english: string) => string;
type LibraryTab = "discover" | "installed" | "studio";

type CatalogModel = {
  id: string;
  name: string;
  source: string;
  author: string;
  license: string;
  licenseWarning: string;
  licenseNote: string;
  repositoryUrl: string;
  licenseUrl?: string | null;
  termsUrl?: string | null;
  description: string;
  category: string;
  compatibilityProfile: string;
  modelKind?: "spine38" | "live2d";
  installed: boolean;
  active: boolean;
};
type PreloadedCatalogModel = {
  modelId: string;
  name: string;
  modelPath: string;
  atlasPath: string | null;
  modelKind: "spine38";
};
type PreloadFailure = { modelId: string; message: string };
type CatalogPreloadResult = { models: PreloadedCatalogModel[]; failures: PreloadFailure[] };
type CatalogLicenseAcceptance = {
  accepted: true;
  modelId: string;
  repositoryUrl: string;
  licenseUrl: string;
  termsUrl: string;
};

type CatalogResult = {
  models: CatalogModel[];
  sources: string[];
  categories: string[];
  total: number;
  page: number;
  pageSize: number;
  totalPages: number;
};
type DownloadProgress = {
  modelId: string;
  completedFiles: number;
  totalFiles: number;
  currentFile: string;
  phase: string;
};
type InstalledModel = {
  id: string;
  name: string;
  modelKind: "spine38" | "live2d";
  source: string;
  license: string;
  path: string;
  atlasPath?: string | null;
  live2dCorePath?: string | null;
  active: boolean;
};

type AvatarPack = {
  id: string;
  name: string;
  path: string;
  runtimeReady?: boolean;
  hasRuntimeExport?: boolean;
  draft?: boolean;
};
type AvatarManifest = {
  id: string;
  name: string;
  source: string;
  licenseNote: string;
  preview?: string;
  layers: Array<Record<string, unknown>>;
  motions: Record<string, string>;
  states: Record<string, string>;
  [key: string]: unknown;
};
type AvatarValidation = {
  ok: boolean;
  id?: string;
  name?: string;
  errors?: string[];
  warnings?: string[];
  runtimeReady?: boolean;
};
type AvatarAssetBytes = { bytes: number[]; mime: string };

const emptyCatalog: CatalogResult = {
  models: [],
  sources: [],
  categories: [],
  total: 0,
  page: 1,
  pageSize: 18,
  totalPages: 1,
};

let catalogResult = emptyCatalog;
let installedModels: InstalledModel[] = [];
let avatarPacks: AvatarPack[] = [];
let activeTab: LibraryTab = "discover";
let query = "";
let source = "";
let category = "";
let progressListenerReady = false;
let catalogRequestVersion = 0;
let avatarSelectionVersion = 0;
let pendingInstallId: string | null = null;
let avatarPreviewUrl: string | null = null;
let libraryTranslate: Translate | null = null;
const dataErrors = { catalog: "", installed: "", avatar: "" };
let installedNotice: { message: string; detail?: string; kind: "success" | "error" } | null = null;
let installedPreviewRenderer: CompanionRenderer | null = null;
let installedPreviewVersion = 0;
let catalogPreloadVersion = 0;
const catalogPreviewFrames = new Map<string, string>();
let catalogCountFrame = 0;
let lastCatalogTotal = 0;

function escapeHtml(value: unknown): string {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function categoryLabel(value: string, tr: Translate): string {
  const labels: Record<string, [string, string]> = {
    operator: ["干员", "Operators"],
    illustration: ["动态立绘", "Illustrations"],
    enemy: ["敌方角色", "Enemies"],
    live2d: ["Live2D 官方样例", "Official Live2D"],
  };
  const label = labels[value];
  return label ? tr(label[0], label[1]) : value;
}

function modelGlyph(name: string): string {
  const value = name.trim();
  if (!value) return "SP";
  if (/[^\u0000-\u007f]/.test(value[0])) return value[0];
  const words = value.split(/[^a-z0-9]+/i).filter(Boolean);
  return (words.length > 1 ? `${words[0][0]}${words[1][0]}` : value.slice(0, 2)).toUpperCase();
}

function setStatus(id: string, message: string, kind = ""): void {
  const element = document.querySelector<HTMLElement>(`#${id}`);
  if (!element) return;
  element.className = `settings-status ${kind}`;
  element.textContent = message;
}

function loadingMarkup(tr: Translate): string {
  return `<section class="companion-library-shell" aria-busy="true">
    <div class="companion-section-head"><div><span class="companion-eyebrow">COMPANION</span><h3>${tr("角色模型", "Character models")}</h3></div></div>
    <div class="companion-model-grid">${Array.from({ length: 6 }, () => `<div class="companion-model-card companion-skeleton"><div></div><span></span><i></i></div>`).join("")}</div>
  </section>`;
}

function tabMarkup(tab: LibraryTab, label: string, count?: number): string {
  return `<button type="button" role="tab" id="companion-tab-${tab}" aria-controls="companion-panel-${tab}" tabindex="${activeTab === tab ? "0" : "-1"}" class="companion-tab ${activeTab === tab ? "active" : ""}" data-native-action="companion-library-tab" data-library-tab="${tab}" aria-selected="${activeTab === tab}">${label}${count === undefined ? "" : `<span>${count}</span>`}</button>`;
}

function modelCard(model: CatalogModel, tr: Translate): string {
  const modelKind = model.modelKind ?? "spine38";
  const previewFrame = catalogPreviewFrames.get(model.id);
  const status = model.active
    ? tr("使用中", "Active")
    : model.installed
      ? tr("已安装", "Installed")
      : tr("可下载", "Available");
  const action = model.installed
    ? `<button class="primary-btn companion-card-action" type="button" data-native-action="companion-activate-model" data-model-id="${escapeHtml(model.id)}" ${model.active ? "disabled" : ""}>${model.active ? tr("正在使用", "In use") : tr("设为伙伴", "Use model")}</button>`
    : `<button class="primary-btn companion-card-action" type="button" data-native-action="companion-request-install" data-model-id="${escapeHtml(model.id)}">${tr("获取模型", "Get model")}</button>`;
  const previewState = modelKind === "live2d"
    ? tr("下载前需同意官方许可", "Official license acceptance required")
    : previewFrame
      ? tr("预览已缓存", "Preview cached")
      : tr("正在生成预览", "Generating preview");
  return `<article class="companion-model-card ${previewFrame ? "has-preview" : ""}" data-model-card="${escapeHtml(model.id)}" title="${escapeHtml(model.description)}">
    <div class="companion-model-visual ${previewFrame ? "has-preview" : "is-empty"} category-${escapeHtml(model.category)}" role="img" aria-label="${escapeHtml(model.name)}">
      ${previewFrame ? `<img src="${previewFrame}" alt="${escapeHtml(model.name)}" />` : `<span class="companion-model-glyph" aria-hidden="true">${escapeHtml(modelGlyph(model.name))}</span>`}<span class="companion-model-runtime">${modelKind === "live2d" ? "LIVE2D" : "SPINE 3.8"}</span><small data-model-preview-state>${previewState}</small>
      <span class="companion-model-status ${model.active ? "active" : ""}">${status}</span>
    </div>
    <div class="companion-model-copy"><div class="companion-model-title"><div><strong>${escapeHtml(model.name)}</strong><span>${escapeHtml(model.author || model.source)}</span></div><span class="companion-kind">${escapeHtml(categoryLabel(model.category, tr))}</span></div>
      <div class="companion-license" title="${escapeHtml(model.licenseWarning || model.licenseNote)}"><span>!</span>${model.license && model.license !== "NOASSERTION" ? escapeHtml(model.license) : tr("授权未声明", "License unverified")}</div>
      <div class="companion-card-foot"><div><a class="text-btn" href="${escapeHtml(model.repositoryUrl)}" target="_blank" rel="noreferrer">${tr("查看来源", "Source")}</a></div>${action}</div>
    </div>
  </article>`;
}

function paginationMarkup(tr: Translate): string {
  if (catalogResult.totalPages <= 1) return "";
  return `<nav class="companion-pagination" aria-label="${tr("模型分页", "Model pages")}">
    <button class="ghost-btn" type="button" data-native-action="companion-catalog-page" data-page="${catalogResult.page - 1}" ${catalogResult.page <= 1 ? "disabled" : ""}>← ${tr("上一页", "Previous")}</button>
    <span>${catalogResult.page} / ${catalogResult.totalPages}</span>
    <button class="ghost-btn" type="button" data-native-action="companion-catalog-page" data-page="${catalogResult.page + 1}" ${catalogResult.page >= catalogResult.totalPages ? "disabled" : ""}>${tr("下一页", "Next")} →</button>
  </nav>`;
}

function catalogMarkup(tr: Translate): string {
  const sourceOptions = catalogResult.sources
    .map((value) => `<option value="${escapeHtml(value)}" ${source === value ? "selected" : ""}>${escapeHtml(value)}</option>`)
    .join("");
  const categoryButtons = ["", ...catalogResult.categories]
    .map((value) => `<button class="companion-filter-chip ${category === value ? "active" : ""}" type="button" data-native-action="companion-catalog-category" data-category="${escapeHtml(value)}">${value ? escapeHtml(categoryLabel(value, tr)) : tr("全部", "All")}</button>`)
    .join("");
  return `<section class="companion-library-panel" id="companion-panel-discover" role="tabpanel" aria-labelledby="companion-tab-discover" data-library-panel="discover">
    <div class="companion-section-head"><div><span class="companion-eyebrow">MODEL GALLERY</span><h3>${tr("挑选一个桌面伙伴", "Choose a desktop companion")}</h3><p>${tr("预览、下载并直接启用。文件只保存在本机，下载后会校验完整性。", "Preview, download, and activate. Files stay local and are integrity-checked.")}</p></div><strong class="companion-catalog-count" id="companionCatalogCount" aria-label="${catalogResult.total.toLocaleString()}">${lastCatalogTotal.toLocaleString()}</strong></div>
    <div class="companion-catalog-toolbar"><label class="companion-search"><span aria-hidden="true">⌕</span><input id="companionCatalogQuery" value="${escapeHtml(query)}" placeholder="${tr("搜索角色、来源或标签", "Search characters, sources, or tags")}" /></label><select class="select" id="companionCatalogSource" aria-label="${tr("模型来源", "Model source")}"><option value="">${tr("全部来源", "All sources")}</option>${sourceOptions}</select><button class="ghost-btn" type="button" data-native-action="companion-search-catalog">${tr("搜索", "Search")}</button></div>
    <div class="companion-filter-row">${categoryButtons}</div>
    <div class="settings-status ${dataErrors.catalog ? "error" : ""}" id="companionCatalogStatus" role="status" aria-live="polite">${dataErrors.catalog ? tr("模型读取失败，请重试。", "Could not load models. Try again.") : ""}</div>
    ${!dataErrors.catalog && catalogResult.models.length ? `<div class="companion-frame-cache-status" id="companionPreloadStatus" role="status" aria-live="polite"><span class="companion-frame-cache-dot" aria-hidden="true"></span><span>${tr("正在为当前页生成模型预览…", "Generating model previews for this page…")}</span></div>` : ""}
    <div class="companion-model-grid">${dataErrors.catalog ? `<div class="companion-empty-state error"><strong>${tr("模型广场暂时不可用", "Model gallery unavailable")}</strong><p>${tr("保留了你的筛选条件，可以直接重试。", "Your filters are preserved; you can retry now.")}</p><details><summary>${tr("技术详情", "Technical details")}</summary><p>${escapeHtml(dataErrors.catalog)}</p></details><button class="ghost-btn" type="button" data-native-action="companion-retry-library">${tr("重试", "Retry")}</button></div>` : catalogResult.models.map((model) => modelCard(model, tr)).join("") || `<div class="companion-empty-state"><span>◇</span><strong>${tr("没有匹配的模型", "No matching models")}</strong><p>${tr("试试更短的关键词或切换分类。", "Try a shorter query or another category.")}</p></div>`}</div>
    ${paginationMarkup(tr)}
  </section>`;
}

function installedMarkup(tr: Translate): string {
  const items = installedModels.map((model) => `<article class="companion-installed-card ${model.active ? "active" : ""}">
    <div class="companion-installed-mark">${model.modelKind === "live2d" ? "L2" : "SP"}</div>
    <div><strong>${escapeHtml(model.name)}</strong><span>${model.modelKind === "live2d" ? "Live2D" : "Spine 3.8"} · ${escapeHtml(model.source)}</span></div>
    <span class="companion-kind">${model.active ? tr("当前伙伴", "Current") : tr("已安装", "Installed")}</span>
    <div class="row"><button class="ghost-btn" type="button" data-native-action="companion-preview-installed" data-model-id="${escapeHtml(model.id)}">${tr("预览", "Preview")}</button>${model.active ? "" : `<button class="primary-btn" type="button" data-native-action="companion-activate-model" data-model-id="${escapeHtml(model.id)}">${tr("设为伙伴", "Use")}</button><button class="text-btn danger" type="button" data-native-action="companion-remove-model" data-model-id="${escapeHtml(model.id)}">${tr("移除", "Remove")}</button>`}</div>
  </article>`).join("");
  return `<section class="companion-library-panel" id="companion-panel-installed" role="tabpanel" aria-labelledby="companion-tab-installed" data-library-panel="installed"><div class="companion-section-head"><div><span class="companion-eyebrow">ON THIS DEVICE</span><h3>${tr("已安装模型", "Installed models")}</h3><p>${tr("切换模型会立即更新桌面伙伴，当前模型受到删除保护。", "Switching updates the companion immediately; the active model is protected.")}</p></div></div>
    <div class="settings-status ${installedNotice?.kind ?? ""}" id="companionInstalledStatus" role="status" aria-live="polite">${installedNotice ? `${escapeHtml(installedNotice.message)}${installedNotice.detail ? `<details><summary>${tr("技术详情", "Technical details")}</summary><p>${escapeHtml(installedNotice.detail)}</p></details>` : ""}` : ""}</div>
    ${installedModels.length ? `<div class="companion-installed-preview"><div class="companion-installed-preview-stage" id="companionInstalledPreviewStage" role="img" aria-label="${tr("已安装模型实时预览", "Installed model live preview")}"><div class="companion-installed-preview-empty" id="companionInstalledPreviewEmpty"><span>SP</span><strong>${tr("选择一个模型进行实时预览", "Choose a model for live preview")}</strong><small>${tr("预览不会切换当前桌面伙伴", "Previewing does not switch your desktop companion")}</small></div><span class="companion-live-badge" id="companionInstalledPreviewBadge">${tr("待预览", "Ready")}</span></div><div><span class="companion-eyebrow">LIVE PREVIEW</span><strong id="companionInstalledPreviewTitle">${tr("已安装模型", "Installed model")}</strong><p>${tr("这里只运行一个预览实例，切换模型时会自动释放上一个渲染器。", "Only one preview runs here; the previous renderer is released when you switch models.")}</p></div></div>` : ""}
    <div class="companion-installed-list">${dataErrors.installed ? `<div class="companion-empty-state error"><strong>${tr("无法读取已安装模型", "Installed models unavailable")}</strong><p>${escapeHtml(dataErrors.installed)}</p><button class="ghost-btn" type="button" data-native-action="companion-retry-library">${tr("重试", "Retry")}</button></div>` : items || `<div class="companion-empty-state"><span>＋</span><strong>${tr("还没有安装模型", "No models installed")}</strong><p>${tr("从模型广场获取，或导入你自己的 Spine / Live2D 模型。", "Get one from the gallery or import your own Spine / Live2D model.")}</p><button class="primary-btn" type="button" data-native-action="companion-import-model">${tr("导入本地模型", "Import local model")}</button></div>`}</div></section>`;
}

function avatarMarkup(tr: Translate): string {
  const options = avatarPacks.map((pack) => `<option value="${escapeHtml(pack.path)}" data-runtime-ready="${pack.runtimeReady === true}">${escapeHtml(pack.name)}${pack.draft ? ` · ${tr("草稿", "Draft")}` : ""}</option>`).join("");
  return `<section class="companion-library-panel" id="companion-panel-studio" role="tabpanel" aria-labelledby="companion-tab-studio" data-library-panel="studio"><div class="companion-section-head"><div><span class="companion-eyebrow">AVATAR STUDIO</span><h3>${tr("制作自己的伙伴", "Create your own companion")}</h3><p>${tr("从素材包开始，导入图层、检查结构，再安装可运行导出。", "Start with a pack, add layers, validate it, then install a runtime export.")}</p></div></div>
    <ol class="avatar-steps"><li><span>1</span><div><strong>${tr("创建形象包", "Create a pack")}</strong><small>${tr("生成标准目录和清单", "Generate structure and manifest")}</small></div></li><li><span>2</span><div><strong>${tr("加入素材", "Add artwork")}</strong><small>${tr("预览并整理图层", "Preview and arrange layers")}</small></div></li><li><span>3</span><div><strong>${tr("校验并启用", "Validate and use")}</strong><small>${tr("需要合法运行时导出", "Requires a legal runtime export")}</small></div></li></ol>
    <div class="avatar-studio-grid"><div class="avatar-studio-form"><div class="field"><label for="avatarParent">${tr("保存位置", "Save location")}</label><div class="row"><input class="input" id="avatarParent" readonly placeholder="${tr("选择一个文件夹", "Choose a folder")}" /><button class="ghost-btn" type="button" data-native-action="avatar-choose-parent">${tr("选择", "Choose")}</button></div></div><div class="settings-grid"><div class="field"><label for="avatarId">ID</label><input class="input" id="avatarId" value="my-avatar" /></div><div class="field"><label for="avatarName">${tr("名称", "Name")}</label><input class="input" id="avatarName" value="${tr("我的桌面伙伴", "My desktop companion")}" /></div></div><button class="primary-btn" type="button" data-native-action="avatar-create-pack">${tr("生成标准形象包", "Generate avatar pack")}</button>
      <div class="avatar-pack-divider"><span>${tr("继续编辑", "Continue editing")}</span></div><div class="field"><label for="avatarPackSelect">${tr("形象包", "Avatar pack")}</label><select class="select" id="avatarPackSelect"><option value="">${tr("选择已登记的形象包", "Choose a registered pack")}</option>${options}</select></div><div class="row avatar-actions"><button class="ghost-btn" type="button" data-native-action="avatar-import-layers" data-requires-avatar-pack disabled>${tr("导入图层", "Import layers")}</button><button class="ghost-btn" type="button" data-native-action="avatar-load-manifest" data-requires-avatar-pack disabled>${tr("刷新预览", "Refresh preview")}</button><button class="primary-btn" type="button" data-native-action="avatar-validate-pack" data-requires-avatar-pack disabled>${tr("检查形象包", "Validate pack")}</button></div></div>
      <div class="avatar-preview-panel"><div class="avatar-preview"><img id="avatarPreview" alt="${tr("形象包预览", "Avatar pack preview")}" hidden /><div id="avatarPreviewEmpty"><span>◇</span><strong>${tr("预览会显示在这里", "Preview appears here")}</strong><small>${tr("先选择形象包并刷新预览", "Choose a pack and refresh the preview")}</small></div></div><button class="primary-btn companion-install-avatar" id="avatarInstallPack" type="button" data-native-action="avatar-install-pack" disabled>${tr("安装并设为伙伴", "Install and use")}</button></div></div>
    <div class="settings-status ${dataErrors.avatar ? "error" : ""}" id="avatarStudioStatus" role="status" aria-live="polite">${escapeHtml(dataErrors.avatar)}</div><details class="avatar-advanced"><summary>${tr("高级：编辑形象包清单", "Advanced: edit pack manifest")}</summary><textarea class="textarea" id="avatarManifestEditor" spellcheck="false" placeholder="avatar-pack.json"></textarea><div class="row"><button class="ghost-btn" type="button" data-native-action="avatar-load-manifest" data-requires-avatar-pack disabled>${tr("加载 JSON", "Load JSON")}</button><button class="ghost-btn" type="button" data-native-action="avatar-save-manifest" data-requires-avatar-pack disabled>${tr("保存 JSON", "Save JSON")}</button></div></details>
  </section>`;
}

function licenseDialogMarkup(tr: Translate): string {
  return `<dialog class="companion-dialog" id="companionLicenseDialog" aria-labelledby="companionLicenseTitle" aria-describedby="companionLicenseNote companionLicenseWarning"><div class="dialog-head"><div><span class="companion-eyebrow">THIRD-PARTY MODEL</span><strong id="companionLicenseTitle"></strong></div><button class="icon-btn" type="button" data-native-action="companion-cancel-install" aria-label="${tr("关闭", "Close")}">×</button></div><div class="dialog-body"><p id="companionLicenseNote"></p><div class="companion-license-links" id="companionLicenseLinks" hidden><a id="companionLicenseUrl" target="_blank" rel="noreferrer">${tr("Free Material License", "Free Material License")}</a><a id="companionTermsUrl" target="_blank" rel="noreferrer">${tr("角色样例条款", "Sample Model Terms")}</a></div><label class="companion-license-acceptance" id="companionLicenseAcceptanceRow" for="companionLicenseAcceptance" hidden><input id="companionLicenseAcceptance" type="checkbox" /><span>${tr("我已阅读并同意以上两份 Live2D 许可条款", "I have read and accept both Live2D license terms above")}</span></label><div class="companion-license-notice"><span>!</span><p id="companionLicenseWarning"></p></div></div><div class="dialog-foot"><button class="ghost-btn" type="button" data-native-action="companion-cancel-install">${tr("取消", "Cancel")}</button><button class="primary-btn" id="companionConfirmInstall" type="button" data-native-action="companion-confirm-install">${tr("了解并下载", "Acknowledge & download")}</button></div></dialog>`;
}

function renderShell(tr: Translate): void {
  const root = document.querySelector<HTMLElement>("#companion-library");
  if (!root) return;
  const preloadVersion = ++catalogPreloadVersion;
  disposeCompanionLibraryPreview();
  catalogPreviewFrames.clear();
  root.innerHTML = `<section class="companion-library-shell"><nav class="companion-tabs" role="tablist">${tabMarkup("discover", tr("模型广场", "Gallery"), catalogResult.total)}${tabMarkup("installed", tr("已安装", "Installed"), installedModels.length)}${tabMarkup("studio", "Avatar Studio")}</nav>${activeTab === "discover" ? catalogMarkup(tr) : activeTab === "installed" ? installedMarkup(tr) : avatarMarkup(tr)}${licenseDialogMarkup(tr)}</section>`;
  bindTabKeyboard(tr);
  bindKeyboardSearch(tr);
  bindAvatarSelection(tr);
  animateCatalogCount();
  if (activeTab === "discover") {
    void cacheCatalogPreviewFrames(preloadVersion, tr);
  } else {
    void invoke<CatalogPreloadResult>("companion_preload_catalog_models", { modelIds: [] }).catch(() => undefined);
  }
}

function updateCatalogCardPreview(modelId: string, frame: string | null, message: string, detail = ""): void {
  const card = document.querySelector<HTMLElement>(`[data-model-card="${CSS.escape(modelId)}"]`);
  if (!card) return;
  const visual = card.querySelector<HTMLElement>(".companion-model-visual");
  const state = card.querySelector<HTMLElement>("[data-model-preview-state]");
  if (state) state.textContent = message;
  if (state) {
    state.hidden = Boolean(frame);
    state.style.display = frame ? "none" : "";
  }
  card.title = detail;
  card.classList.toggle("has-preview", Boolean(frame));
  card.classList.toggle("preview-failed", !frame);
  visual?.classList.toggle("has-preview", Boolean(frame));
  visual?.classList.toggle("is-empty", !frame);
  if (!frame || !visual) return;
  catalogPreviewFrames.set(modelId, frame);
  visual.querySelector(".companion-model-glyph")?.remove();
  let image = visual.querySelector<HTMLImageElement>("img");
  if (!image) {
    image = document.createElement("img");
    image.alt = card.querySelector<HTMLElement>(".companion-model-title strong")?.textContent ?? "";
    visual.prepend(image);
  }
  image.src = frame;
}

async function captureCatalogFrame(
  stage: HTMLElement,
  model: PreloadedCatalogModel,
  version: number,
): Promise<string | null> {
  let instance: CompanionRenderer | null = null;
  try {
    const { companionRenderer } = await import("./renderer");
    if (version !== catalogPreloadVersion || activeTab !== "discover") return null;
    stage.replaceChildren();
    instance = companionRenderer(stage, {
      enabled: false,
      modelPath: model.modelPath,
      atlasPath: model.atlasPath,
      modelName: model.name,
      modelKind: model.modelKind,
      live2dCorePath: null,
      scale: 0.82,
      bubbleVisible: false,
    });
    await instance.init();
    if (version !== catalogPreloadVersion || activeTab !== "discover") return null;
    instance.applyPhase("idle");
    await new Promise<void>((resolve) => window.requestAnimationFrame(() => window.requestAnimationFrame(() => resolve())));
    return instance.captureFrame?.() ?? null;
  } finally {
    instance?.destroy();
    stage.replaceChildren();
  }
}

async function cacheCatalogPreviewFrames(version: number, tr: Translate): Promise<void> {
  const candidates = catalogResult.models.filter((model) => (model.modelKind ?? "spine38") === "spine38");
  if (!candidates.length) {
    await invoke<CatalogPreloadResult>("companion_preload_catalog_models", { modelIds: [] }).catch(() => undefined);
    if (version !== catalogPreloadVersion) return;
    setStatus("companionPreloadStatus", tr("当前页没有可生成预览的 Spine 模型。", "No Spine models on this page can be previewed."));
    return;
  }
  const root = document.querySelector<HTMLElement>("#companion-library");
  if (!root) return;
  const stage = document.createElement("div");
  stage.className = "companion-capture-stage";
  stage.setAttribute("aria-hidden", "true");
  root.appendChild(stage);
  const installedById = new Map(installedModels.map((model) => [model.id, model]));
  const installed = candidates.filter((model) => model.installed);
  const downloadable = candidates.filter((model) => !model.installed);
  let completed = 0;
  let failed = 0;
  const reportProgress = (): void => {
    setStatus("companionPreloadStatus", tr(`正在生成当前页预览 ${completed + failed}/${candidates.length}…`, `Generating previews ${completed + failed}/${candidates.length}…`));
  };
  const capture = async (model: PreloadedCatalogModel): Promise<void> => {
    if (version !== catalogPreloadVersion || activeTab !== "discover") return;
    try {
      const frame = await captureCatalogFrame(stage, model, version);
      if (version !== catalogPreloadVersion || activeTab !== "discover") return;
      if (!frame) throw new Error("renderer returned a blank frame");
      completed += 1;
      updateCatalogCardPreview(model.modelId, frame, tr("预览已缓存", "Preview cached"));
    } catch (error) {
      if (version !== catalogPreloadVersion) return;
      failed += 1;
      updateCatalogCardPreview(model.modelId, null, tr("预览生成失败", "Preview unavailable"), String(error));
    }
    reportProgress();
  };

  try {
    reportProgress();
    for (const model of installed) {
      const local = installedById.get(model.id);
      if (!local || local.modelKind !== "spine38") {
        failed += 1;
        updateCatalogCardPreview(model.id, null, tr("预览生成失败", "Preview unavailable"));
        reportProgress();
        continue;
      }
      await capture({
        modelId: local.id,
        name: local.name,
        modelPath: local.path,
        atlasPath: local.atlasPath ?? null,
        modelKind: "spine38",
      });
    }
    for (let index = 0; index < downloadable.length; index += 4) {
      if (version !== catalogPreloadVersion || activeTab !== "discover") return;
      const batch = downloadable.slice(index, index + 4);
      let result: CatalogPreloadResult;
      try {
        result = await invoke<CatalogPreloadResult>("companion_preload_catalog_models", { modelIds: batch.map((model) => model.id) });
      } catch (error) {
        for (const model of batch) {
          failed += 1;
          updateCatalogCardPreview(model.id, null, tr("模型缓存失败", "Model cache failed"), String(error));
        }
        reportProgress();
        continue;
      }
      if (version !== catalogPreloadVersion || activeTab !== "discover") return;
      for (const failure of result.failures) {
        failed += 1;
        updateCatalogCardPreview(failure.modelId, null, tr("模型缓存失败", "Model cache failed"), failure.message);
      }
      for (const model of result.models) await capture(model);
      const reported = new Set([...result.models.map((model) => model.modelId), ...result.failures.map((failure) => failure.modelId)]);
      for (const model of batch) {
        if (reported.has(model.id)) continue;
        failed += 1;
        updateCatalogCardPreview(model.id, null, tr("模型缓存失败", "Model cache failed"));
      }
      reportProgress();
    }
    if (version !== catalogPreloadVersion || activeTab !== "discover") return;
    setStatus("companionPreloadStatus", failed
      ? tr(`已缓存 ${completed}/${candidates.length} 个模型预览，${failed} 个模型暂不兼容。`, `${completed}/${candidates.length} previews cached; ${failed} models are not currently compatible.`)
      : tr(`当前页 ${completed} 个模型预览已缓存。`, `${completed} model previews cached for this page.`), failed ? "" : "success");
  } catch (error) {
    if (version !== catalogPreloadVersion) return;
    setStatus("companionPreloadStatus", tr("模型预览生成失败，不影响模型安装。", "Preview generation failed; model installation is still available."), "error");
    const status = document.querySelector<HTMLElement>("#companionPreloadStatus");
    if (status) status.title = String(error);
  } finally {
    stage.remove();
    if (version === catalogPreloadVersion && activeTab === "discover") {
      void invoke<CatalogPreloadResult>("companion_preload_catalog_models", { modelIds: [] }).catch(() => undefined);
    }
  }
}

function animateCatalogCount(): void {
  const element = document.querySelector<HTMLElement>("#companionCatalogCount");
  if (!element) return;
  const target = catalogResult.total;
  const start = lastCatalogTotal;
  lastCatalogTotal = target;
  window.cancelAnimationFrame(catalogCountFrame);
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches || start === target) {
    element.textContent = target.toLocaleString();
    return;
  }
  const startedAt = performance.now();
  const duration = 720;
  element.classList.add("is-counting");
  const tick = (now: number): void => {
    if (!element.isConnected) return;
    const progress = Math.min(1, (now - startedAt) / duration);
    const eased = 1 - Math.pow(1 - progress, 3);
    element.textContent = Math.round(start + (target - start) * eased).toLocaleString();
    if (progress < 1) catalogCountFrame = window.requestAnimationFrame(tick);
    else element.classList.remove("is-counting");
  };
  catalogCountFrame = window.requestAnimationFrame(tick);
}

export function disposeCompanionLibraryPreview(): void {
  installedPreviewVersion += 1;
  installedPreviewRenderer?.destroy();
  installedPreviewRenderer = null;
}

function bindTabKeyboard(tr: Translate): void {
  const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>(".companion-tabs [role=\"tab\"]"));
  tabs.forEach((tab, index) => tab.addEventListener("keydown", (event) => {
    if (!(["ArrowLeft", "ArrowRight", "Home", "End"] as string[]).includes(event.key)) return;
    event.preventDefault();
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? tabs.length - 1
        : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    activeTab = tabs[nextIndex].dataset.libraryTab as LibraryTab;
    renderShell(tr);
    document.querySelector<HTMLButtonElement>(`[data-library-tab="${activeTab}"]`)?.focus();
  }));
}

function bindAvatarSelection(tr: Translate): void {
  const select = document.querySelector<HTMLSelectElement>("#avatarPackSelect");
  if (!select) return;
  const update = (): void => {
    const hasPack = Boolean(select.value);
    document.querySelectorAll<HTMLButtonElement>("[data-requires-avatar-pack]").forEach((button) => { button.disabled = !hasPack; });
    const install = document.querySelector<HTMLButtonElement>("#avatarInstallPack");
    if (install) {
      install.disabled = !hasPack || select.selectedOptions[0]?.dataset.runtimeReady !== "true";
      install.title = install.disabled && hasPack ? tr("需要可运行的 Spine 导出", "Runtime-ready Spine export required") : "";
    }
  };
  select.addEventListener("change", () => {
    const version = ++avatarSelectionVersion;
    clearAvatarEditorAndPreview();
    document.querySelectorAll<HTMLButtonElement>("[data-requires-avatar-pack]").forEach((button) => { button.disabled = true; });
    const install = document.querySelector<HTMLButtonElement>("#avatarInstallPack");
    if (install) install.disabled = true;
    if (!select.value) {
      update();
      return;
    }
    void loadAvatarSelection(select.value, version, tr, update);
  });
  document.querySelector<HTMLTextAreaElement>("#avatarManifestEditor")?.addEventListener("input", () => {
    invalidateAvatarReadiness(tr, tr("清单已修改，需要重新检查。", "Manifest changed; validate it again."));
  });
  update();
}

function clearAvatarEditorAndPreview(): void {
  const editor = document.querySelector<HTMLTextAreaElement>("#avatarManifestEditor");
  if (editor) editor.value = "";
  const preview = document.querySelector<HTMLImageElement>("#avatarPreview");
  const empty = document.querySelector<HTMLElement>("#avatarPreviewEmpty");
  if (avatarPreviewUrl) {
    URL.revokeObjectURL(avatarPreviewUrl);
    avatarPreviewUrl = null;
  }
  if (preview) {
    preview.removeAttribute("src");
    preview.hidden = true;
  }
  if (empty) empty.hidden = false;
}

async function loadAvatarSelection(packPath: string, version: number, tr: Translate, finish: () => void): Promise<void> {
  setStatus("avatarStudioStatus", tr("正在加载形象包…", "Loading avatar pack…"));
  try {
    const manifest = await invoke<AvatarManifest>("companion_load_avatar_manifest", { input: { path: packPath } });
    if (version !== avatarSelectionVersion) return;
    const editor = document.querySelector<HTMLTextAreaElement>("#avatarManifestEditor");
    if (editor) editor.value = JSON.stringify(manifest, null, 2);
    await loadAvatarPreview(packPath, manifest, () => version === avatarSelectionVersion);
    if (version !== avatarSelectionVersion) return;
    finish();
    setStatus("avatarStudioStatus", tr("形象包已加载，可以预览或继续编辑。", "Avatar pack loaded; you can preview or continue editing."), "success");
  } catch (error) {
    if (version !== avatarSelectionVersion) return;
    setStatus("avatarStudioStatus", tr("形象包加载失败，请重试。", "Could not load the avatar pack. Try again."), "error");
    const status = document.querySelector<HTMLElement>("#avatarStudioStatus");
    if (status) status.title = String(error);
  }
}

function invalidateAvatarReadiness(tr: Translate, message?: string): void {
  const install = document.querySelector<HTMLButtonElement>("#avatarInstallPack");
  if (install) {
    install.disabled = true;
    install.title = tr("需要重新检查形象包", "Validate the avatar pack again");
  }
  const selected = document.querySelector<HTMLSelectElement>("#avatarPackSelect")?.selectedOptions[0];
  if (selected) selected.dataset.runtimeReady = "false";
  if (message) setStatus("avatarStudioStatus", message);
}

function bindKeyboardSearch(tr: Translate): void {
  const input = document.querySelector<HTMLInputElement>("#companionCatalogQuery");
  input?.addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    void searchCatalog(1, tr);
  });
}

async function ensureProgressListener(tr: Translate): Promise<void> {
  if (progressListenerReady) return;
  try {
    await listen<DownloadProgress>("companion:model-download-progress", ({ payload }) => {
    const translate = libraryTranslate ?? tr;
    const card = document.querySelector<HTMLElement>(`[data-model-card="${CSS.escape(payload.modelId)}"]`);
    const button = card?.querySelector<HTMLButtonElement>(".companion-card-action");
    if (button) {
      button.disabled = payload.phase !== "failed";
      button.textContent = payload.phase === "installed"
        ? translate("安装完成", "Installed")
        : payload.phase === "failed"
          ? translate("重试", "Retry")
          : `${translate("下载中", "Downloading")} ${payload.completedFiles}/${payload.totalFiles}`;
    }
    setStatus("companionCatalogStatus", payload.phase === "installed" ? translate("模型已通过校验并安装。", "Model verified and installed.") : payload.phase === "failed" ? `${translate("下载失败", "Download failed")}: ${payload.currentFile}` : `${translate("正在下载", "Downloading")} ${payload.currentFile}`, payload.phase === "failed" ? "error" : "");
    });
    progressListenerReady = true;
  } catch (error) {
    progressListenerReady = false;
    throw error;
  }
}

async function refreshData(page: number): Promise<void> {
  const [catalog, installed, avatars] = await Promise.allSettled([
    invoke<CatalogResult>("companion_search_catalog", { query: query || null, source: source || null, category: category || null, page }),
    invoke<InstalledModel[]>("companion_list_installed_models"),
    invoke<AvatarPack[]>("companion_list_avatar_packs"),
  ]);
  if (catalog.status === "fulfilled") { catalogResult = catalog.value; dataErrors.catalog = ""; }
  else {
    catalogResult = { ...catalogResult, models: [], total: 0, page, totalPages: 1 };
    dataErrors.catalog = String(catalog.reason);
  }
  if (installed.status === "fulfilled") { installedModels = installed.value; dataErrors.installed = ""; }
  else dataErrors.installed = String(installed.reason);
  if (avatars.status === "fulfilled") {
    avatarPacks = avatars.value.map((pack) => ({ ...pack, draft: pack.draft ?? pack.runtimeReady === false }));
    dataErrors.avatar = "";
  } else dataErrors.avatar = String(avatars.reason);
}

async function searchCatalog(page: number, tr: Translate): Promise<void> {
  const requestVersion = ++catalogRequestVersion;
  query = document.querySelector<HTMLInputElement>("#companionCatalogQuery")?.value.trim() ?? query;
  source = document.querySelector<HTMLSelectElement>("#companionCatalogSource")?.value ?? source;
  setStatus("companionCatalogStatus", tr("正在整理模型…", "Loading models…"));
  const panel = document.querySelector<HTMLElement>("[data-library-panel=\"discover\"]");
  panel?.setAttribute("aria-busy", "true");
  document.querySelectorAll<HTMLButtonElement>("[data-native-action=\"companion-catalog-page\"]").forEach((item) => { item.disabled = true; });
  try {
    const next = await invoke<CatalogResult>("companion_search_catalog", { query: query || null, source: source || null, category: category || null, page });
    if (requestVersion !== catalogRequestVersion) return;
    catalogResult = next;
    dataErrors.catalog = "";
    renderShell(tr);
  } catch (error) {
    if (requestVersion !== catalogRequestVersion) return;
    catalogResult = { ...catalogResult, models: [], total: 0, page, totalPages: 1 };
    dataErrors.catalog = String(error);
    renderShell(tr);
  }
}

export async function renderCompanionLibrary(tr: Translate): Promise<void> {
  libraryTranslate = tr;
  const root = document.querySelector<HTMLElement>("#companion-library");
  if (!root) return;
  root.innerHTML = loadingMarkup(tr);
  try {
    await ensureProgressListener(tr);
    await refreshData(1);
    renderShell(tr);
  } catch (error) {
    root.innerHTML = `<div class="companion-empty-state error"><strong>${tr("模型库暂时不可用", "Model library unavailable")}</strong><p>${escapeHtml(error)}</p><button class="ghost-btn" type="button" data-native-action="companion-retry-library">${tr("重试", "Retry")}</button></div>`;
  }
}

function formatValidation(validation: AvatarValidation, tr: Translate): string {
  if (validation.ok) return validation.runtimeReady ? tr("检查通过，可以安装并启用。", "Validation passed; ready to install.") : tr("草稿结构有效；仍需要合法的 Spine 运行时导出。", "Draft is valid; a legal Spine runtime export is still required.");
  return (validation.errors ?? [tr("形象包检查未通过。", "Avatar pack validation failed.")]).join(" · ");
}

function applyAvatarRuntimeReadiness(validation: AvatarValidation): void {
  const ready = validation.ok && validation.runtimeReady === true;
  const install = document.querySelector<HTMLButtonElement>("#avatarInstallPack");
  if (install) install.disabled = !ready;
  const selected = document.querySelector<HTMLSelectElement>("#avatarPackSelect")?.selectedOptions[0];
  if (selected) selected.dataset.runtimeReady = String(ready);
}

async function previewInstalledModel(model: InstalledModel, tr: Translate): Promise<void> {
  disposeCompanionLibraryPreview();
  const version = installedPreviewVersion;
  const stage = document.querySelector<HTMLElement>("#companionInstalledPreviewStage");
  const empty = document.querySelector<HTMLElement>("#companionInstalledPreviewEmpty");
  const badge = document.querySelector<HTMLElement>("#companionInstalledPreviewBadge");
  const title = document.querySelector<HTMLElement>("#companionInstalledPreviewTitle");
  if (!stage) return;
  if (model.modelKind === "live2d" && !model.live2dCorePath) {
    setStatus("companionInstalledStatus", tr("Live2D Core 未通过校验，无法预览该模型。", "The validated Live2D Core is unavailable, so this model cannot be previewed."), "error");
    return;
  }
  if (badge) badge.textContent = tr("正在加载", "Loading");
  if (title) title.textContent = model.name;
  const settings: CompanionSettings = {
    enabled: false,
    modelPath: model.path,
    atlasPath: model.atlasPath ?? null,
    modelName: model.name,
    modelKind: model.modelKind,
    live2dCorePath: model.live2dCorePath ?? null,
    scale: 0.85,
    bubbleVisible: false,
  };
  let instance: CompanionRenderer | null = null;
  try {
    const { companionRenderer } = await import("./renderer");
    if (version !== installedPreviewVersion || !stage.isConnected) return;
    instance = companionRenderer(stage, settings);
    await instance.init();
    if (version !== installedPreviewVersion || !stage.isConnected) {
      instance.destroy();
      return;
    }
    instance.applyPhase("idle");
    installedPreviewRenderer = instance;
    if (empty) empty.hidden = true;
    stage.classList.add("ready");
    if (badge) badge.textContent = tr("实时预览", "Live preview");
    setStatus("companionInstalledStatus", tr("预览已加载；当前桌面伙伴没有改变。", "Preview loaded; your active desktop companion was not changed."), "success");
  } catch (error) {
    instance?.destroy();
    if (version !== installedPreviewVersion) return;
    if (badge) badge.textContent = tr("加载失败", "Load failed");
    setStatus("companionInstalledStatus", tr("模型预览失败，请查看详情后重试。", "Model preview failed. Review the details and try again."), "error");
    const status = document.querySelector<HTMLElement>("#companionInstalledStatus");
    if (status) status.title = String(error);
  }
}

async function installCatalogModel(model: CatalogModel, tr: Translate, licenseAcceptance: CatalogLicenseAcceptance | null): Promise<void> {
  setStatus("companionCatalogStatus", tr("正在下载并校验模型…", "Downloading and verifying model…"));
  try {
    await invoke("companion_install_catalog_model", { modelId: model.id, licenseAcceptance });
  } catch (error) {
    await refreshData(catalogResult.page);
    renderShell(tr);
    setStatus("companionCatalogStatus", String(error), "error");
    return;
  }
  await refreshData(catalogResult.page);
  try {
    await invoke("companion_activate_installed_model", { modelId: model.id });
    await refreshData(catalogResult.page);
    renderShell(tr);
    setStatus("companionCatalogStatus", tr("模型已安装并设为当前伙伴。", "Model installed and activated."), "success");
    window.dispatchEvent(new CustomEvent("companion:refresh-settings"));
  } catch (error) {
    await refreshData(catalogResult.page);
    renderShell(tr);
    setStatus("companionCatalogStatus", tr("模型已安装，但暂时无法启用；可在“已安装”中重试。", "The model is installed but could not be activated. Retry from Installed."), "error");
    const status = document.querySelector<HTMLElement>("#companionCatalogStatus");
    if (status) status.title = String(error);
  }
}

export async function handleCompanionLibraryAction(action: string, tr: Translate, button?: HTMLButtonElement): Promise<boolean> {
  if (action === "companion-retry-library") { void renderCompanionLibrary(tr); return true; }
  if (action === "companion-library-tab") {
    activeTab = (button?.dataset.libraryTab as LibraryTab | undefined) ?? "discover";
    renderShell(tr);
    return true;
  }
  if (action === "companion-search-catalog") { await searchCatalog(1, tr); return true; }
  if (action === "companion-catalog-category") {
    category = button?.dataset.category ?? "";
    await searchCatalog(1, tr);
    return true;
  }
  if (action === "companion-catalog-page") {
    await searchCatalog(Number(button?.dataset.page ?? 1), tr);
    return true;
  }
  if (action === "companion-request-install") {
    const model = catalogResult.models.find((item) => item.id === button?.dataset.modelId);
    if (!model) return true;
    pendingInstallId = model.id;
    document.querySelector<HTMLElement>("#companionLicenseTitle")!.textContent = model.name;
    document.querySelector<HTMLElement>("#companionLicenseNote")!.textContent = model.licenseNote;
    document.querySelector<HTMLElement>("#companionLicenseWarning")!.textContent = model.licenseWarning || tr("请确认你有权使用该第三方模型。", "Confirm that you are allowed to use this third-party model.");
    const licenseLinks = document.querySelector<HTMLElement>("#companionLicenseLinks");
    const licenseUrl = document.querySelector<HTMLAnchorElement>("#companionLicenseUrl");
    const termsUrl = document.querySelector<HTMLAnchorElement>("#companionTermsUrl");
    const acceptanceRow = document.querySelector<HTMLElement>("#companionLicenseAcceptanceRow");
    const acceptance = document.querySelector<HTMLInputElement>("#companionLicenseAcceptance");
    const confirm = document.querySelector<HTMLButtonElement>("#companionConfirmInstall");
    const hasLicenseLinks = Boolean(model.licenseUrl && model.termsUrl);
    const requiresAcceptance = (model.modelKind ?? "spine38") === "live2d";
    if (licenseLinks) licenseLinks.hidden = !hasLicenseLinks;
    if (licenseUrl && model.licenseUrl) licenseUrl.href = model.licenseUrl;
    if (termsUrl && model.termsUrl) termsUrl.href = model.termsUrl;
    if (acceptanceRow) acceptanceRow.hidden = !requiresAcceptance;
    if (acceptance) {
      acceptance.checked = false;
      acceptance.onchange = () => { if (confirm) confirm.disabled = requiresAcceptance && !acceptance.checked; };
    }
    if (confirm) {
      confirm.disabled = requiresAcceptance;
      confirm.textContent = requiresAcceptance ? tr("同意并下载", "Accept & download") : tr("了解并下载", "Acknowledge & download");
    }
    document.querySelector<HTMLDialogElement>("#companionLicenseDialog")?.showModal();
    return true;
  }
  if (action === "companion-cancel-install") {
    pendingInstallId = null;
    document.querySelector<HTMLDialogElement>("#companionLicenseDialog")?.close();
    return true;
  }
  if (action === "companion-confirm-install") {
    const modelId = pendingInstallId;
    const model = catalogResult.models.find((item) => item.id === modelId);
    const requiresAcceptance = (model?.modelKind ?? "spine38") === "live2d";
    const accepted = document.querySelector<HTMLInputElement>("#companionLicenseAcceptance")?.checked === true;
    if (requiresAcceptance && !accepted) return true;
    const licenseAcceptance: CatalogLicenseAcceptance | null = requiresAcceptance && model?.licenseUrl && model.termsUrl
      ? {
          accepted: true,
          modelId: model.id,
          repositoryUrl: model.repositoryUrl,
          licenseUrl: model.licenseUrl,
          termsUrl: model.termsUrl,
        }
      : null;
    pendingInstallId = null;
    document.querySelector<HTMLDialogElement>("#companionLicenseDialog")?.close();
    if (model) await installCatalogModel(model, tr, licenseAcceptance);
    return true;
  }
  if (action === "companion-preview-installed") {
    const model = installedModels.find((item) => item.id === button?.dataset.modelId);
    if (model) await previewInstalledModel(model, tr);
    return true;
  }
  if (action === "companion-activate-model" || action === "companion-remove-model") {
    const id = button?.dataset.modelId;
    if (!id) return true;
    if (action === "companion-remove-model" && !window.confirm(tr("确认移除这个本地模型？", "Remove this local model?"))) return true;
    try {
      if (action === "companion-activate-model") await invoke("companion_activate_installed_model", { modelId: id });
      else await invoke("companion_remove_installed_model", { modelId: id });
      await refreshData(catalogResult.page);
      installedNotice = { message: action === "companion-activate-model" ? tr("伙伴已切换。", "Companion changed.") : tr("模型已从本机移除。", "Model removed from this device."), kind: "success" };
      renderShell(tr);
      if (action === "companion-activate-model") window.dispatchEvent(new CustomEvent("companion:refresh-settings"));
    } catch (error) {
      installedNotice = { message: tr("操作失败，请查看详情后重试。", "The action failed. Review the details and try again."), detail: String(error), kind: "error" };
      renderShell(tr);
    }
    return true;
  }
  if (action === "avatar-choose-parent") {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") document.querySelector<HTMLInputElement>("#avatarParent")!.value = selected;
    return true;
  }
  if (action === "avatar-create-pack") {
    const parent = document.querySelector<HTMLInputElement>("#avatarParent")?.value;
    const id = document.querySelector<HTMLInputElement>("#avatarId")?.value.trim();
    const name = document.querySelector<HTMLInputElement>("#avatarName")?.value.trim();
    if (!parent || !id || !name) { setStatus("avatarStudioStatus", tr("请填写保存位置、ID 和名称。", "Choose a location and enter an ID and name."), "error"); return true; }
    setStatus("avatarStudioStatus", tr("正在生成标准形象包…", "Generating avatar pack…"));
    try {
      const created = await invoke<{ path: string }>("companion_create_avatar_pack", { input: { path: `${parent}\\${id}`, id, name, source: "local", licenseNote: tr("用户自有或获授权素材", "User-owned or licensed assets") } });
      await invoke("companion_register_avatar_pack", { input: { path: created.path } });
      await refreshData(catalogResult.page);
      activeTab = "studio";
      renderShell(tr);
      const select = document.querySelector<HTMLSelectElement>("#avatarPackSelect");
      if (select) select.value = created.path;
      select?.dispatchEvent(new Event("change"));
      setStatus("avatarStudioStatus", tr("草稿已生成。接下来导入图层并加入合法运行时导出。", "Draft generated. Add layers and a legal runtime export next."), "success");
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  const packPath = document.querySelector<HTMLSelectElement>("#avatarPackSelect")?.value;
  if (action.startsWith("avatar-") && !packPath) { setStatus("avatarStudioStatus", tr("请先选择一个形象包。", "Choose an avatar pack first."), "error"); return true; }
  if (action === "avatar-import-layers") {
    try {
      const selected = await open({ multiple: true, directory: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }] });
      if (!selected || typeof selected === "string") return true;
      await invoke("companion_import_avatar_layers", { input: { packPath, files: selected } });
      invalidateAvatarReadiness(tr);
      const manifest = await invoke<AvatarManifest>("companion_load_avatar_manifest", { input: { path: packPath } });
      const editor = document.querySelector<HTMLTextAreaElement>("#avatarManifestEditor");
      if (editor) editor.value = JSON.stringify(manifest, null, 2);
      await loadAvatarPreview(packPath!, manifest);
      setStatus("avatarStudioStatus", tr(`已导入 ${selected.length} 个图层，请重新检查形象包。`, `${selected.length} layers imported; validate the avatar pack again.`), "success");
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  if (action === "avatar-load-manifest") {
    try {
      const manifest = await invoke<AvatarManifest>("companion_load_avatar_manifest", { input: { path: packPath } });
      const editor = document.querySelector<HTMLTextAreaElement>("#avatarManifestEditor");
      if (editor) editor.value = JSON.stringify(manifest, null, 2);
      await loadAvatarPreview(packPath!, manifest);
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  if (action === "avatar-save-manifest") {
    try {
      const manifest = JSON.parse(document.querySelector<HTMLTextAreaElement>("#avatarManifestEditor")?.value || "{}");
      const validation = await invoke<AvatarValidation>("companion_save_avatar_manifest", { input: { path: packPath }, manifest });
      applyAvatarRuntimeReadiness(validation);
      setStatus("avatarStudioStatus", formatValidation(validation, tr), validation.ok ? "success" : "error");
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  if (action === "avatar-validate-pack") {
    try {
      const validation = await invoke<AvatarValidation>("companion_validate_avatar_pack", { input: { path: packPath } });
      applyAvatarRuntimeReadiness(validation);
      setStatus("avatarStudioStatus", formatValidation(validation, tr), validation.ok ? "success" : "error");
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  if (action === "avatar-install-pack") {
    setStatus("avatarStudioStatus", tr("正在检查运行时导出…", "Checking runtime export…"));
    try {
      const installed = await invoke<{ modelId: string }>("companion_install_avatar_pack", { input: { path: packPath } });
      await invoke("companion_activate_installed_model", { modelId: installed.modelId });
      await refreshData(catalogResult.page);
      activeTab = "installed";
      installedNotice = { message: tr("形象包已安装并设为当前伙伴。", "Avatar pack installed and activated."), kind: "success" };
      renderShell(tr);
      window.dispatchEvent(new CustomEvent("companion:refresh-settings"));
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  return false;
}

async function loadAvatarPreview(packPath: string, manifest: AvatarManifest, isCurrent: () => boolean = () => true): Promise<void> {
  const preview = document.querySelector<HTMLImageElement>("#avatarPreview");
  const empty = document.querySelector<HTMLElement>("#avatarPreviewEmpty");
  if (!preview) return;
  try {
    const asset = await invoke<AvatarAssetBytes>("companion_read_avatar_asset", { input: { path: packPath }, relativePath: typeof manifest.preview === "string" ? manifest.preview : "preview.png" });
    if (!isCurrent()) return;
    if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
    avatarPreviewUrl = URL.createObjectURL(new Blob([new Uint8Array(asset.bytes)], { type: asset.mime }));
    preview.src = avatarPreviewUrl; preview.hidden = false; if (empty) empty.hidden = true;
  } catch {
    if (!isCurrent()) return;
    preview.removeAttribute("src"); preview.hidden = true; if (empty) empty.hidden = false;
  }
}
