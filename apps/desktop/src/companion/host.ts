import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import type {
  CompanionActivity,
  CompanionPhase,
  CompanionSettings,
  ExecutionActivity,
} from "./types";
import {
  disposeCompanionLibraryPreview,
  handleCompanionLibraryAction,
  renderCompanionLibrary,
} from "./library";
import type { CompanionRenderer } from "./renderer";

type Translate = (chinese: string, english: string) => string;
type RendererStatus = { state: "disabled" | "ready" | "error"; message: string };

let settingsPreview: CompanionRenderer | null = null;
let currentTranslate: Translate | null = null;
let rendererStatusListenerReady = false;

export function disposeCompanionSettingsPreview(): void {
  settingsPreview?.destroy();
  settingsPreview = null;
  disposeCompanionLibraryPreview();
}

const failedPattern = /failed|failure|error|失败|错误/i;
const waitingPattern = /paused|waiting|approval|cancel|暂停|等待|审批|取消/i;
const reviewPattern = /review|verify|reason|analy|检查|验证|推理|分析/i;
const runningPattern = /running tool|tool|command|执行|运行工具|命令/i;

export function companionPhaseForActivity(
  activity: ExecutionActivity,
): CompanionPhase {
  const state = `${activity.state} ${activity.detail}`;
  if (failedPattern.test(state)) return "failed";
  if (waitingPattern.test(state)) return "waiting";
  if (reviewPattern.test(state)) return "reviewing";
  if (runningPattern.test(state)) return "running";
  if (!activity.running) return "success";
  return "working";
}

export function syncCompanionActivity(activity: ExecutionActivity): void {
  if (!isTauri()) return;
  const payload: CompanionActivity = {
    phase: companionPhaseForActivity(activity),
    title: activity.workerId
      ? `${activity.role} · ${activity.workerId}`
      : activity.role,
    detail: `${activity.state} · ${activity.detail}`,
  };
  void invoke("companion_set_activity", { activity: payload }).catch(() => {
    // Companion rendering must never change a run's outcome.
  });
}

function checked(selector: string, fallback: boolean): boolean {
  return (
    document.querySelector<HTMLInputElement>(selector)?.checked ?? fallback
  );
}

function scaleValue(fallback: number): number {
  const value = Number(
    document.querySelector<HTMLInputElement>("#companionScale")?.value,
  );
  return Number.isFinite(value) ? value : fallback;
}

function settingsMarkup(settings: CompanionSettings, tr: Translate): string {
  const configured = Boolean(settings.modelPath);
  return `
    <section class="companion-settings-hero ${configured ? "configured" : "empty"}">
      <div class="companion-live-preview"><div id="companionSettingsPreview" aria-label="${tr("当前桌面伙伴预览", "Current companion preview")}"></div>${configured ? "" : `<div class="companion-live-empty"><span>◇</span><strong>${tr("选择你的第一个伙伴", "Choose your first companion")}</strong><small>${tr("支持 Spine 3.8 与 Live2D", "Spine 3.8 and Live2D supported")}</small></div>`}<span class="companion-live-badge" id="companionRendererBadge">${settings.enabled ? tr("正在连接", "Connecting") : tr("未显示", "Hidden")}</span></div>
      <div class="companion-hero-copy"><span class="companion-eyebrow">DESKTOP COMPANION</span><h2>${settings.modelName ? escapeText(settings.modelName) : tr("让任务状态更有生命力", "Bring task progress to life")}</h2><p>${configured ? tr("伙伴会用动作回应思考、执行、审查、等待与完成状态。", "Your companion reacts to thinking, running, reviewing, waiting, and completion.") : tr("从模型广场选一个角色，或导入你拥有使用权的模型。", "Choose a character from the gallery or import a model you can legally use.")}</p><div class="companion-current-meta">${configured ? `<span>${settings.modelKind === "live2d" ? "Live2D" : "Spine 3.8"}</span><span>${settings.enabled ? tr("桌面显示已开启", "Visible on desktop") : tr("桌面显示已关闭", "Desktop display off")}</span>` : `<span>${tr("尚未配置模型", "No model configured")}</span>`}</div><div class="row"><button class="primary-btn" type="button" data-native-action="companion-import-model">${configured ? tr("导入其他模型", "Import another") : tr("导入本地模型", "Import local model")}</button>${configured ? `<button class="ghost-btn" type="button" data-native-action="companion-jump-gallery">${tr("浏览模型广场", "Browse gallery")}</button>` : ""}</div></div>
    </section>
    <section class="companion-preferences"><div class="companion-section-head"><div><span class="companion-eyebrow">BEHAVIOR</span><h3>${tr("显示与反馈", "Display & feedback")}</h3></div><button class="primary-btn" type="button" data-native-action="companion-save">${tr("保存设置", "Save settings")}</button></div>
      <div class="companion-preference-grid"><label class="companion-preference" for="companionEnabled"><div><strong>${tr("显示桌面伙伴", "Show desktop companion")}</strong><span>${configured ? tr("使用独立透明窗口，不启动额外服务。", "Uses a separate transparent window with no extra service.") : tr("请先选择一个模型。", "Choose a model first.")}</span></div><input id="companionEnabled" aria-label="${tr("显示桌面伙伴", "Show desktop companion")}" type="checkbox" ${settings.enabled ? "checked" : ""} ${configured ? "" : "disabled"} /></label>
      <label class="companion-preference companion-scale-setting" for="companionScale"><div><strong>${tr("角色缩放", "Character scale")}</strong><span id="companionScaleValue">${Math.round(settings.scale * 100)}%</span></div><input id="companionScale" aria-label="${tr("角色缩放", "Character scale")}" type="range" min="0.35" max="1.55" step="0.05" value="${settings.scale}" /></label>
      <label class="companion-preference" for="companionBubbleVisible"><div><strong>${tr("任务气泡", "Task bubble")}</strong><span>${tr("显示当前 Agent 阶段和简短进度。", "Shows the current Agent phase and concise progress.")}</span></div><input id="companionBubbleVisible" aria-label="${tr("任务气泡", "Task bubble")}" type="checkbox" ${settings.bubbleVisible ? "checked" : ""} /></label></div>
      <div class="settings-status" id="companionSettingsStatus" role="status" aria-live="polite"></div>
    </section>`;
}

function escapeText(value: string): string {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
}

async function initSettingsPreview(settings: CompanionSettings, tr: Translate): Promise<void> {
  disposeCompanionSettingsPreview();
  if (!settings.modelPath) return;
  const stage = document.querySelector<HTMLElement>("#companionSettingsPreview");
  const badge = document.querySelector<HTMLElement>("#companionRendererBadge");
  if (!stage) return;
  const { companionRenderer } = await import("./renderer");
  const instance = companionRenderer(stage, { ...settings, scale: Math.min(settings.scale, 1) });
  try {
    await instance.init();
    if (!stage.isConnected) { instance.destroy(); return; }
    instance.applyPhase("idle");
    settingsPreview = instance;
    stage.parentElement?.classList.add("ready");
    if (badge) badge.textContent = settings.enabled ? tr("运行正常", "Ready") : tr("预览", "Preview");
  } catch (error) {
    instance.destroy();
    stage.parentElement?.classList.add("failed");
    if (badge) badge.textContent = tr("加载失败", "Load failed");
    status(`${tr("模型预览失败", "Model preview failed")}: ${String(error)}`, "error");
  }
}

async function ensureRendererStatusListener(tr: Translate): Promise<void> {
  if (rendererStatusListenerReady) return;
  rendererStatusListenerReady = true;
  await listen<RendererStatus>("companion:renderer-status", ({ payload }) => {
    const badge = document.querySelector<HTMLElement>("#companionRendererBadge");
    if (badge) badge.textContent = payload.state === "ready" ? tr("运行正常", "Ready") : payload.state === "error" ? tr("加载失败", "Load failed") : tr("未显示", "Hidden");
    if (payload.state === "error") status(payload.message, "error");
  });
}

export async function renderCompanionSettings(tr: Translate): Promise<void> {
  currentTranslate = tr;
  const panel = document.querySelector<HTMLElement>("#nativeSettingsPanel");
  if (!panel) return;
  if (!isTauri()) {
    panel.innerHTML = `<div class="pane-head" style="padding-inline:0"><strong>${tr("桌面伙伴", "Desktop companion")}</strong></div><span class="settings-help">${tr("此功能仅在 LunaScope 桌面版中可用。", "This feature is available in the LunaScope desktop app.")}</span>`;
    return;
  }
  panel.innerHTML = `<div class="pane-head" style="padding-inline:0"><strong>${tr("桌面伙伴", "Desktop companion")}</strong></div><span class="settings-help">${tr("正在读取本地设置…", "Loading local settings…")}</span>`;
  try {
    disposeCompanionSettingsPreview();
    const settings = await invoke<CompanionSettings>("companion_get_settings");
    panel.innerHTML = `${settingsMarkup(settings, tr)}<div id="companion-library"></div>`;
    document.querySelector<HTMLInputElement>("#companionScale")?.addEventListener("input", (event) => {
      const value = Number((event.currentTarget as HTMLInputElement).value);
      const output = document.querySelector<HTMLElement>("#companionScaleValue");
      if (output) output.textContent = `${Math.round(value * 100)}%`;
    });
    await ensureRendererStatusListener(tr);
    void initSettingsPreview(settings, tr);
    void renderCompanionLibrary(tr);
  } catch (error) {
    panel.innerHTML = `<div class="pane-head" style="padding-inline:0"><strong>${tr("桌面伙伴", "Desktop companion")}</strong></div><div class="settings-status error">${String(error)}</div>`;
  }
}

function status(message: string, kind = ""): void {
  const element = document.querySelector<HTMLElement>(
    "#companionSettingsStatus",
  );
  if (!element) return;
  element.className = `settings-status ${kind}`;
  element.textContent = message;
}

export async function handleCompanionSettingsAction(
  action: string,
  tr: Translate,
  button?: HTMLButtonElement,
): Promise<boolean> {
  if (!action.startsWith("companion-") && !action.startsWith("avatar-")) return false;
  if (await handleCompanionLibraryAction(action, tr, button)) return true;
  if (action === "companion-jump-gallery") {
    document.querySelector<HTMLButtonElement>('[data-library-tab="discover"]')?.click();
    document.querySelector<HTMLElement>("#companion-library")?.scrollIntoView({ behavior: "smooth", block: "start" });
    return true;
  }
  if (action === "companion-import-model") {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: "Spine 3.8 / Live2D", extensions: ["skel", "json"] },
      ],
    });
    if (!selected || Array.isArray(selected)) return true;
    status(
      tr(
        "正在导入本地模型；Live2D 首次导入会从官方地址获取 Cubism Core…",
        "Importing the local model; the first Live2D import fetches Cubism Core from the official source…",
      ),
    );
    try {
      await invoke<CompanionSettings>("companion_import_model", {
        sourcePath: selected,
      });
      await renderCompanionSettings(tr);
      status(tr("模型已导入并启用。", "Model imported and enabled."), "success");
    } catch (error) {
      status(String(error), "error");
    }
    return true;
  }
  if (action === "companion-save") {
    try {
      const current = await invoke<CompanionSettings>("companion_get_settings");
      await invoke<CompanionSettings>("companion_save_preferences", {
        preferences: {
          enabled: checked("#companionEnabled", current.enabled),
          scale: scaleValue(current.scale),
          bubbleVisible: checked(
            "#companionBubbleVisible",
            current.bubbleVisible,
          ),
        },
      });
      await renderCompanionSettings(tr);
      status(tr("桌面伙伴设置已保存。", "Companion settings saved."), "success");
    } catch (error) {
      status(String(error), "error");
    }
    return true;
  }
  return true;
}

window.addEventListener("companion:refresh-settings", () => {
  if (currentTranslate) void renderCompanionSettings(currentTranslate);
});
