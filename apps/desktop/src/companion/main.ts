import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import "./styles.css";
import {
  CompanionPointerGesture,
  placeBubbleNearModel,
  shouldShowCompanionBubble,
} from "./interaction";
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
let settingsGeneration = 0;
const pointerGesture = new CompanionPointerGesture();

function hideBubble(): void {
  window.clearTimeout(bubbleTimer);
  bubbleTimer = 0;
  if (bubble) bubble.hidden = true;
}

function positionBubble(): void {
  if (!stage || !bubble || bubble.hidden) return;
  const bounds = renderer?.getInteractiveBounds();
  if (!bounds) {
    bubble.style.left = "12px";
    bubble.style.top = "12px";
    bubble.dataset.side = "above";
    return;
  }
  const placement = placeBubbleNearModel(
    { width: stage.clientWidth, height: stage.clientHeight },
    { width: bubble.offsetWidth, height: bubble.offsetHeight },
    bounds,
  );
  bubble.style.left = `${Math.round(placement.left)}px`;
  bubble.style.top = `${Math.round(placement.top)}px`;
  bubble.dataset.side = placement.side;
}

function positionHideButton(): void {
  if (!stage || !hideButton) return;
  const bounds = renderer?.getInteractiveBounds();
  if (!bounds) {
    hideButton.style.removeProperty("left");
    hideButton.style.removeProperty("right");
    hideButton.style.removeProperty("top");
    return;
  }
  const left = Math.min(
    stage.clientWidth - hideButton.offsetWidth - 8,
    Math.max(8, bounds.right - hideButton.offsetWidth * 0.4),
  );
  const top = Math.min(
    stage.clientHeight - hideButton.offsetHeight - 8,
    Math.max(8, bounds.top - hideButton.offsetHeight * 0.4),
  );
  hideButton.style.left = `${Math.round(left)}px`;
  hideButton.style.right = "auto";
  hideButton.style.top = `${Math.round(top)}px`;
}

function positionOverlays(): void {
  positionBubble();
  positionHideButton();
}

function stagePoint(event: PointerEvent): { x: number; y: number } | null {
  if (!stage) return null;
  const rect = stage.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}

function releasePointer(pointerId: number): void {
  try {
    if (stage?.hasPointerCapture(pointerId)) stage.releasePointerCapture(pointerId);
  } catch {
    // Native window dragging may release capture before the DOM receives cleanup.
  }
}

function showError(message: string): void {
  if (!errorElement) return;
  errorElement.textContent = message;
  errorElement.hidden = false;
}

async function applySettings(next: CompanionSettings): Promise<void> {
  const generation = ++settingsGeneration;
  settings = next;
  hideBubble();
  pointerGesture.cancel();
  renderer?.destroy();
  renderer = null;
  if (!next.enabled || !next.modelPath) {
    await appWindow.hide();
    void invoke("companion_report_renderer_status", {
      status: { state: "disabled", message: "" },
    }).catch(() => undefined);
    return;
  }
  if (!stage) return;
  errorElement?.setAttribute("hidden", "");
  const instance = companionRenderer(stage, next);
  try {
    await instance.init();
    if (generation !== settingsGeneration) {
      instance.destroy();
      return;
    }
    instance.applyPhase("idle");
    renderer = instance;
    await appWindow.show();
    window.requestAnimationFrame(positionOverlays);
    void invoke("companion_report_renderer_status", {
      status: { state: "ready", message: "" },
    }).catch(() => undefined);
  } catch (error) {
    instance.destroy();
    if (generation !== settingsGeneration) return;
    const message = `模型加载失败：${String(error)}`;
    console.error(message);
    showError(message);
    void invoke("companion_report_renderer_status", {
      status: { state: "error", message },
    }).catch(() => undefined);
  }
}

function applyActivity(activity: CompanionActivity): void {
  renderer?.applyPhase(activity.phase);
  if (
    !bubble ||
    !title ||
    !detail ||
    !shouldShowCompanionBubble(Boolean(settings?.bubbleVisible), activity)
  ) {
    hideBubble();
    return;
  }
  title.textContent = activity.title;
  detail.textContent = activity.detail;
  bubble.hidden = false;
  window.requestAnimationFrame(positionOverlays);
  window.clearTimeout(bubbleTimer);
  bubbleTimer = window.setTimeout(() => {
    hideBubble();
  }, activity.phase === "success" || activity.phase === "failed" ? 8000 : 5000);
}

stage?.addEventListener("pointerdown", (event) => {
  const point = stagePoint(event);
  if (!point || !renderer?.containsPoint(point.x, point.y)) return;
  if (!pointerGesture.begin({ pointerId: event.pointerId, button: event.button, ...point })) return;
  event.preventDefault();
  stage.setPointerCapture(event.pointerId);
});

stage?.addEventListener("pointermove", (event) => {
  const point = stagePoint(event);
  if (!point) return;
  hideButton?.classList.toggle("is-near-model", Boolean(renderer?.containsPoint(point.x, point.y)));
  if (pointerGesture.move({ pointerId: event.pointerId, ...point }) !== "start-drag") return;
  releasePointer(event.pointerId);
  void appWindow.startDragging().finally(() => pointerGesture.cancel(event.pointerId));
});

stage?.addEventListener("pointerup", (event) => {
  const point = stagePoint(event);
  const outcome = point
    ? pointerGesture.end({ pointerId: event.pointerId, ...point })
    : "none";
  releasePointer(event.pointerId);
  if (outcome === "interact") renderer?.playInteraction();
});

stage?.addEventListener("pointercancel", (event) => {
  pointerGesture.cancel(event.pointerId);
  releasePointer(event.pointerId);
});

stage?.addEventListener("pointerleave", () => {
  hideButton?.classList.remove("is-near-model");
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
window.addEventListener("resize", () => window.requestAnimationFrame(positionOverlays));

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
