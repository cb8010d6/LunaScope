import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

type Translate = (chinese: string, english: string) => string;

type CatalogModel = {
  id: string;
  name: string;
  source: string;
  author: string;
  license: string;
  licenseWarning: string;
  licenseNote: string;
  repositoryUrl: string;
  description: string;
  category: string;
  compatibilityProfile: string;
  installed: boolean;
  active: boolean;
};

type CatalogResult = { models: CatalogModel[]; sources: string[]; total: number };
type InstalledModel = {
  id: string;
  name: string;
  modelKind: "spine38" | "live2d";
  source: string;
  license: string;
  path: string;
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

let catalogResult: CatalogResult = { models: [], sources: [], total: 0 };
let installedModels: InstalledModel[] = [];
let avatarPacks: AvatarPack[] = [];

function escapeHtml(value: unknown): string {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function setStatus(id: string, message: string, kind = ""): void {
  const element = document.querySelector<HTMLElement>(`#${id}`);
  if (!element) return;
  element.className = `settings-status ${kind}`;
  element.textContent = message;
}

function catalogMarkup(tr: Translate): string {
  const cards = catalogResult.models
    .map(
      (model) => `<article class="provider-card companion-model-card">
        <div class="pane-head" style="padding-inline:0"><div><strong>${escapeHtml(model.name)}</strong><span>${escapeHtml(model.source)} · ${escapeHtml(model.category)}</span></div><span class="provenance">${model.installed ? model.active ? tr("当前", "Active") : tr("已安装", "Installed") : tr("可下载", "Available")}</span></div>
        <small>${escapeHtml(model.author)} · ${escapeHtml(model.license || "NOASSERTION")}</small>
        <p class="settings-help">${escapeHtml(model.description || model.licenseNote)}</p>
        <p class="settings-help" style="color:var(--warning, #d99b3d)">${escapeHtml(model.licenseWarning || model.licenseNote)}</p>
        <div class="row"><a class="text-btn" href="${escapeHtml(model.repositoryUrl)}" target="_blank" rel="noreferrer">${tr("来源", "Source")}</a>${model.installed ? `<button class="ghost-btn" type="button" data-native-action="companion-activate-model" data-model-id="${escapeHtml(model.id)}" ${model.active ? "disabled" : ""}>${model.active ? tr("当前使用", "Active") : tr("启用", "Use")}</button>` : `<button class="primary-btn" type="button" data-native-action="companion-install-model" data-model-id="${escapeHtml(model.id)}">${tr("下载并安装", "Download & install")}</button>`}</div>
      </article>`,
    )
    .join("");
  const sources = catalogResult.sources
    .map((source) => `<option value="${escapeHtml(source)}">${escapeHtml(source)}</option>`)
    .join("");
  return `<section class="companion-library-section"><div class="pane-head" style="padding-inline:0"><div><strong>${tr("模型库", "Model library")}</strong><span class="settings-help">${tr("目录只包含元数据；模型文件按需下载到本机并校验摘要。", "Catalogs contain metadata only; model files are downloaded on demand and digest-verified locally.")}</span></div><span class="mono">${catalogResult.total}</span></div>
    <div class="row"><input class="input" id="companionCatalogQuery" placeholder="${tr("搜索名称、来源或标签", "Search name, source, or tag")}" /><select class="select" id="companionCatalogSource"><option value="">${tr("全部来源", "All sources")}</option>${sources}</select><button class="ghost-btn" type="button" data-native-action="companion-search-catalog">${tr("搜索", "Search")}</button></div>
    <div class="settings-status" id="companionCatalogStatus" role="status"></div><div class="companion-model-grid">${cards || `<span class="settings-help">${tr("没有匹配的模型。", "No matching models.")}</span>`}</div></section>`;
}

function installedMarkup(tr: Translate): string {
  const items = installedModels
    .map(
      (model) => `<div class="setting-row"><div><strong>${escapeHtml(model.name)}</strong><span>${model.modelKind === "live2d" ? "Live2D" : "Spine 3.8"} · ${escapeHtml(model.source)}${model.active ? ` · ${tr("当前", "Active")}` : ""}</span></div><div class="row">${model.active ? "" : `<button class="ghost-btn" type="button" data-native-action="companion-activate-model" data-model-id="${escapeHtml(model.id)}">${tr("启用", "Use")}</button><button class="text-btn" type="button" data-native-action="companion-remove-model" data-model-id="${escapeHtml(model.id)}">${tr("删除", "Remove")}</button>`}</div></div>`,
    )
    .join("");
  return `<section class="companion-library-section"><div class="pane-head" style="padding-inline:0"><div><strong>${tr("已安装", "Installed")}</strong><span class="settings-help">${tr("当前使用中的模型不能删除。", "The active model cannot be removed.")}</span></div></div>${items || `<span class="settings-help">${tr("尚未安装模型。", "No models installed.")}</span>`}</section>`;
}

function avatarMarkup(tr: Translate): string {
  const options = avatarPacks
    .map((pack) => `<option value="${escapeHtml(pack.path)}">${escapeHtml(pack.name)} · ${escapeHtml(pack.id)}${pack.draft ? ` · ${tr("草稿", "Draft")}` : ""}</option>`)
    .join("");
  return `<section class="companion-library-section" id="companionAvatarStudio"><div class="pane-head" style="padding-inline:0"><div><strong>Avatar Studio</strong><span class="settings-help">${tr("自动创建标准形象包、装配图层并校验；没有合法 Spine 导出时只会生成可编辑草稿。", "Create standard packs, assemble layers, and validate them; without a legal Spine export this remains an editable draft.")}</span></div></div>
    <div class="settings-grid"><div class="field"><label for="avatarParent">${tr("父目录", "Parent folder")}</label><div class="row"><input class="input" id="avatarParent" readonly placeholder="${tr("选择父目录", "Choose parent folder")}" /><button class="ghost-btn" type="button" data-native-action="avatar-choose-parent">${tr("选择", "Choose")}</button></div></div><div class="field"><label for="avatarId">ID</label><input class="input" id="avatarId" value="my-avatar" /></div><div class="field"><label for="avatarName">${tr("名称", "Name")}</label><input class="input" id="avatarName" value="${tr("我的桌面伙伴", "My desktop companion")}" /></div></div>
    <div class="row"><button class="primary-btn" type="button" data-native-action="avatar-create-pack">${tr("自动创建标准包", "Create standard pack")}</button><select class="select" id="avatarPackSelect"><option value="">${tr("选择已登记形象包", "Choose registered pack")}</option>${options}</select><button class="ghost-btn" type="button" data-native-action="avatar-import-layers">${tr("导入图层", "Import layers")}</button><button class="ghost-btn" type="button" data-native-action="avatar-validate-pack">${tr("校验", "Validate")}</button><button class="primary-btn" type="button" data-native-action="avatar-install-pack">${tr("安装并启用", "Install & enable")}</button></div>
    <div class="avatar-studio-workspace"><div class="avatar-preview"><span class="settings-help">${tr("预览", "Preview")}</span><img id="avatarPreview" alt="" hidden /></div><textarea class="textarea" id="avatarManifestEditor" spellcheck="false" style="min-height:180px" placeholder="${tr("选择形象包后，清单会显示在这里，可直接编辑。", "Select a pack to edit its manifest here.")}"></textarea></div><div class="row"><button class="ghost-btn" type="button" data-native-action="avatar-load-manifest">${tr("加载清单", "Load manifest")}</button><button class="ghost-btn" type="button" data-native-action="avatar-save-manifest">${tr("保存清单", "Save manifest")}</button></div><div class="settings-status" id="avatarStudioStatus" role="status"></div></section>`;
}

async function loadAvatarPacks(): Promise<void> {
  const packs = await invoke<AvatarPack[]>("companion_list_avatar_packs");
  avatarPacks = packs.map((pack) => ({
    ...pack,
    draft: pack.draft ?? pack.runtimeReady === false,
  }));
}

export async function renderCompanionLibrary(tr: Translate): Promise<void> {
  const root = document.querySelector<HTMLElement>("#companion-library");
  if (!root) return;
  try {
    [catalogResult, installedModels] = await Promise.all([
      invoke<CatalogResult>("companion_search_catalog", { query: null, source: null }),
      invoke<InstalledModel[]>("companion_list_installed_models"),
    ]);
    await loadAvatarPacks();
    root.innerHTML = `${catalogMarkup(tr)}${installedMarkup(tr)}${avatarMarkup(tr)}`;
  } catch (error) {
    root.innerHTML = `<div class="settings-status error">${escapeHtml(error)}</div>`;
  }
}

export async function handleCompanionLibraryAction(
  action: string,
  tr: Translate,
  button?: HTMLButtonElement,
): Promise<boolean> {
  if (action === "companion-search-catalog") {
    const query = document.querySelector<HTMLInputElement>("#companionCatalogQuery")?.value ?? "";
    const source = document.querySelector<HTMLSelectElement>("#companionCatalogSource")?.value || null;
    catalogResult = await invoke<CatalogResult>("companion_search_catalog", { query, source });
    installedModels = await invoke<InstalledModel[]>("companion_list_installed_models");
    const root = document.querySelector<HTMLElement>("#companion-library");
    if (root) root.innerHTML = `${catalogMarkup(tr)}${installedMarkup(tr)}${avatarMarkup(tr)}`;
    return true;
  }
  if (action === "companion-install-model" || action === "companion-activate-model" || action === "companion-remove-model") {
    const id = button?.dataset.modelId;
    if (!id) return true;
    const current = catalogResult.models.find((item) => item.id === id);
    if (action === "companion-install-model" && current && !window.confirm(`${current.name}\n\n${current.licenseNote}\n\n${current.licenseWarning}`)) return true;
    if (action === "companion-remove-model" && !window.confirm(tr("确认删除这个已安装模型？", "Remove this installed model?"))) return true;
    setStatus("companionCatalogStatus", tr("正在处理模型…", "Processing model…"));
    try {
      if (action === "companion-install-model") await invoke("companion_install_catalog_model", { modelId: id });
      else if (action === "companion-activate-model") await invoke("companion_activate_installed_model", { modelId: id });
      else await invoke("companion_remove_installed_model", { modelId: id });
      const root = document.querySelector<HTMLElement>("#companion-library");
      if (root) await renderCompanionLibrary(tr);
      setStatus("companionCatalogStatus", tr("模型状态已更新。", "Model state updated."), "success");
    } catch (error) {
      setStatus("companionCatalogStatus", String(error), "error");
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
    if (!parent || !id || !name) return true;
    setStatus("avatarStudioStatus", tr("正在创建标准形象包…", "Creating standard avatar pack…"));
    try {
      await invoke("companion_create_avatar_pack", { input: { path: `${parent}\\${id}`, id, name, source: "local", licenseNote: tr("用户自有或获授权素材", "User-owned or licensed assets") } });
      await renderCompanionLibrary(tr);
      setStatus("avatarStudioStatus", tr("标准包已创建；当前是可编辑草稿。", "Standard pack created; it is an editable draft."), "success");
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  const packPath = document.querySelector<HTMLSelectElement>("#avatarPackSelect")?.value;
  if (action.startsWith("avatar-") && !packPath) { setStatus("avatarStudioStatus", tr("请先选择形象包。", "Choose an avatar pack first."), "error"); return true; }
  if (action === "avatar-import-layers") {
    const selected = await open({ multiple: true, directory: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }] });
    if (!selected || typeof selected === "string") return true;
    await invoke("companion_import_avatar_layers", { input: { packPath, files: selected } });
    await renderCompanionLibrary(tr);
    return true;
  }
  if (action === "avatar-load-manifest") {
    if (!packPath) return true;
    const manifest = await invoke<AvatarManifest>("companion_load_avatar_manifest", { input: { path: packPath } });
    document.querySelector<HTMLTextAreaElement>("#avatarManifestEditor")!.value = JSON.stringify(manifest, null, 2);
    await loadAvatarPreview(packPath, manifest);
    return true;
  }
  if (action === "avatar-save-manifest") {
    try {
      const manifest = JSON.parse(document.querySelector<HTMLTextAreaElement>("#avatarManifestEditor")?.value || "{}");
      const validation = await invoke("companion_save_avatar_manifest", { input: { path: packPath }, manifest });
      setStatus("avatarStudioStatus", JSON.stringify(validation), "success");
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  if (action === "avatar-validate-pack") {
    const validation = await invoke<AvatarValidation>("companion_validate_avatar_pack", { input: { path: packPath } });
    setStatus("avatarStudioStatus", JSON.stringify(validation), validation.ok ? "success" : "error");
    return true;
  }
  if (action === "avatar-install-pack") {
    setStatus("avatarStudioStatus", tr("正在检查运行时导出…", "Checking runtime export…"));
    try {
      const installed = await invoke<{ validation: { id: string } }>("companion_install_avatar_pack", { input: { path: packPath } });
      await invoke("companion_activate_installed_model", { modelId: installed.validation.id });
      setStatus("avatarStudioStatus", tr("形象包已安装并启用。", "Avatar pack installed and enabled."), "success");
      await renderCompanionLibrary(tr);
    } catch (error) { setStatus("avatarStudioStatus", String(error), "error"); }
    return true;
  }
  return false;
}

async function loadAvatarPreview(packPath: string, manifest: AvatarManifest): Promise<void> {
  const preview = document.querySelector<HTMLImageElement>("#avatarPreview");
  if (!preview) return;
  const relativePath = typeof manifest.preview === "string" ? manifest.preview : "preview.png";
  try {
    const asset = await invoke<AvatarAssetBytes>("companion_read_avatar_asset", {
      input: { path: packPath },
      relativePath,
    });
    const blob = new Blob([new Uint8Array(asset.bytes)], { type: asset.mime });
    const previous = preview.dataset.objectUrl;
    if (previous) URL.revokeObjectURL(previous);
    const objectUrl = URL.createObjectURL(blob);
    preview.dataset.objectUrl = objectUrl;
    preview.src = objectUrl;
    preview.hidden = false;
  } catch {
    preview.removeAttribute("src");
    preview.hidden = true;
  }
}
