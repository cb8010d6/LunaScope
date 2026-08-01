import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import "./styles.css";
import type { CompanionRenderer } from "./renderer";
import { companionRenderer } from "./renderer";
import type { CompanionActivity, CompanionSettings } from "./types";

const stage = document.querySelector<HTMLElement>("#companion-stage");
const bubble = document.querySelector<HTMLElement>("#companion-bubble");
const title = document.querySelector<HTMLElement>("#companion-title");
const detail = document.querySelector<HTMLElement>("#companion-detail");
const errorElement = document.querySelector<HTMLElement>("#companion-error");
const hideButton = document.querySelector<HTMLButtonElement>("#companion-hide");
const appWindow = getCurrentWindow();

let renderer: CompanionRenderer | null = null;
let settings: CompanionSettings | null = null;
let bubbleTimer = 0;

function showError(message: string): void {
  if (!errorElement) return;
  errorElement.textContent = message;
  errorElement.hidden = false;
}

async function applySettings(next: CompanionSettings): Promise<void> {
  settings = next;
  if (!next.enabled || !next.modelPath) {
    await appWindow.hide();
    return;
  }
  renderer?.destroy();
  renderer = null;
  if (!stage) return;
  errorElement?.setAttribute("hidden", "");
  const instance = companionRenderer(stage, next);
  try {
    await instance.init();
    instance.applyPhase("idle");
    renderer = instance;
    await appWindow.show();
  } catch (error) {
    instance.destroy();
    showError(`Spine 模型加载失败：${String(error)}`);
  }
}

function applyActivity(activity: CompanionActivity): void {
  renderer?.applyPhase(activity.phase);
  if (!settings?.bubbleVisible || !bubble || !title || !detail) return;
  title.textContent = activity.title;
  detail.textContent = activity.detail;
  bubble.hidden = false;
  window.clearTimeout(bubbleTimer);
  bubbleTimer = window.setTimeout(() => {
    bubble.hidden = true;
  }, activity.phase === "success" || activity.phase === "failed" ? 8000 : 5000);
}

stage?.addEventListener("pointerdown", (event) => {
  if (event.button === 0) void appWindow.startDragging();
});

hideButton?.addEventListener("click", async () => {
  if (!settings) return;
  settings = await invoke<CompanionSettings>("companion_save_preferences", {
    preferences: {
      enabled: false,
      scale: settings.scale,
      bubbleVisible: settings.bubbleVisible,
    },
  });
});

window.addEventListener("contextmenu", (event) => event.preventDefault());

await listen<CompanionActivity>("companion:activity", (event) => {
  applyActivity(event.payload);
});
await listen<CompanionSettings>("companion:settings", (event) => {
  void applySettings(event.payload);
});

try {
  await applySettings(
    await invoke<CompanionSettings>("companion_get_settings"),
  );
} catch (error) {
  showError(String(error));
}
