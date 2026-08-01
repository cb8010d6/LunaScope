import { invoke, isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import type {
  CompanionActivity,
  CompanionPhase,
  CompanionSettings,
  ExecutionActivity,
} from "./types";
import {
  handleCompanionLibraryAction,
  renderCompanionLibrary,
} from "./library";

type Translate = (chinese: string, english: string) => string;

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
    <div class="pane-head" style="padding-inline:0"><div><strong>${tr("桌面伙伴", "Desktop companion")}</strong><span class="settings-help">${tr("内置的 Spine 3.8 伙伴会直接跟随 LunaScope 任务状态。", "The built-in Spine 3.8 companion follows LunaScope task state directly.")}</span></div></div>
    <div class="setting-row">
      <div><strong>${tr("显示桌面伙伴", "Show desktop companion")}</strong><span>${configured ? tr("独立透明窗口，不启动额外服务。", "A separate transparent window with no extra service.") : tr("请先导入一个本地 Spine 3.8 模型。", "Import a local Spine 3.8 model first.")}</span></div>
      <input id="companionEnabled" type="checkbox" ${settings.enabled ? "checked" : ""} ${configured ? "" : "disabled"} />
    </div>
    <div class="setting-row">
      <div><strong>${tr("本地模型", "Local model")}</strong><span>${settings.modelName ? `${settings.modelName} · ${settings.modelKind === "live2d" ? "Live2D" : "Spine 3.8"}` : tr("尚未配置", "Not configured")}</span></div>
      <button class="ghost-btn" type="button" data-native-action="companion-import-model">${tr("选择模型", "Choose model")}</button>
    </div>
    <div class="setting-row">
      <div><strong>${tr("角色缩放", "Character scale")}</strong><span>${Math.round(settings.scale * 100)}%</span></div>
      <input id="companionScale" type="range" min="0.35" max="1.55" step="0.05" value="${settings.scale}" />
    </div>
    <div class="setting-row">
      <div><strong>${tr("任务气泡", "Task bubble")}</strong><span>${tr("显示当前 Agent 阶段和简短进度。", "Show the current Agent phase and concise progress.")}</span></div>
      <input id="companionBubbleVisible" type="checkbox" ${settings.bubbleVisible ? "checked" : ""} />
    </div>
    <div class="row" style="margin-top:14px"><button class="primary-btn" type="button" data-native-action="companion-save">${tr("保存桌面伙伴设置", "Save companion settings")}</button></div>
    <div class="settings-status" id="companionSettingsStatus" role="status"></div>`;
}

export async function renderCompanionSettings(tr: Translate): Promise<void> {
  const panel = document.querySelector<HTMLElement>("#nativeSettingsPanel");
  if (!panel) return;
  if (!isTauri()) {
    panel.innerHTML = `<div class="pane-head" style="padding-inline:0"><strong>${tr("桌面伙伴", "Desktop companion")}</strong></div><span class="settings-help">${tr("此功能仅在 LunaScope 桌面版中可用。", "This feature is available in the LunaScope desktop app.")}</span>`;
    return;
  }
  panel.innerHTML = `<div class="pane-head" style="padding-inline:0"><strong>${tr("桌面伙伴", "Desktop companion")}</strong></div><span class="settings-help">${tr("正在读取本地设置…", "Loading local settings…")}</span>`;
  try {
    const settings = await invoke<CompanionSettings>("companion_get_settings");
    panel.innerHTML = `${settingsMarkup(settings, tr)}<div id="companion-library"></div>`;
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
