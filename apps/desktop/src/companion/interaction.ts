import type { CompanionActivity } from "./types";
import type { CompanionPhase } from "./types";

type PointerStart = {
  pointerId: number;
  button: number;
  x: number;
  y: number;
};

type PointerPosition = Omit<PointerStart, "button">;

export type PointerGestureOutcome = "none" | "start-drag" | "interact";

export class CompanionPointerGesture {
  private readonly thresholdPx: number;
  private active: {
    pointerId: number;
    startX: number;
    startY: number;
    dragging: boolean;
  } | null = null;

  constructor(thresholdPx = 6) {
    this.thresholdPx = thresholdPx;
  }

  begin(pointer: PointerStart): boolean {
    if (pointer.button !== 0 || this.active) return false;
    this.active = {
      pointerId: pointer.pointerId,
      startX: pointer.x,
      startY: pointer.y,
      dragging: false,
    };
    return true;
  }

  move(pointer: PointerPosition): PointerGestureOutcome {
    const active = this.active;
    if (!active || active.pointerId !== pointer.pointerId || active.dragging) return "none";
    if (Math.hypot(pointer.x - active.startX, pointer.y - active.startY) < this.thresholdPx) {
      return "none";
    }
    active.dragging = true;
    return "start-drag";
  }

  end(pointer: PointerPosition): PointerGestureOutcome {
    const active = this.active;
    if (!active || active.pointerId !== pointer.pointerId) return "none";
    this.active = null;
    const moved = Math.hypot(pointer.x - active.startX, pointer.y - active.startY);
    return !active.dragging && moved < this.thresholdPx ? "interact" : "none";
  }

  cancel(pointerId?: number): void {
    if (pointerId === undefined || this.active?.pointerId === pointerId) this.active = null;
  }
}

export function shouldShowCompanionBubble(
  enabled: boolean,
  activity: CompanionActivity,
): boolean {
  return enabled && activity.phase !== "idle" && activity.detail.trim().length > 0;
}

const live2DInteractionCandidates = ["TapBody", "Interact", "Special", "TapHead"];

export function selectLive2DInteractionMotion(groups: string[]): string | null {
  for (const candidate of live2DInteractionCandidates) {
    const match = groups.find((group) => group.toLowerCase() === candidate.toLowerCase());
    if (match) return match;
  }
  return null;
}

export async function startLive2DOneShot(
  startMotion: () => Promise<boolean>,
  onStarted: () => void,
  onUnavailable: () => void,
): Promise<void> {
  try {
    if (await startMotion()) onStarted();
    else onUnavailable();
  } catch {
    onUnavailable();
  }
}

export class CompanionPhaseGate {
  private appliedPhase: CompanionPhase | null = null;
  private currentPhaseValue: CompanionPhase;
  private interactionVersion = 0;
  private activeInteraction: number | null = null;

  constructor(initialPhase: CompanionPhase = "idle") {
    this.currentPhaseValue = initialPhase;
  }

  requestPhase(phase: CompanionPhase): boolean {
    this.currentPhaseValue = phase;
    if (this.activeInteraction !== null || this.appliedPhase === phase) return false;
    this.appliedPhase = phase;
    return true;
  }

  beginInteraction(): number {
    const version = ++this.interactionVersion;
    this.activeInteraction = version;
    return version;
  }

  finishInteraction(version: number): CompanionPhase | null {
    if (this.activeInteraction !== version) return null;
    this.activeInteraction = null;
    this.appliedPhase = this.currentPhaseValue;
    return this.currentPhaseValue;
  }

  cancelInteraction(version: number): void {
    if (this.activeInteraction === version) this.activeInteraction = null;
  }

  reset(): void {
    this.interactionVersion += 1;
    this.activeInteraction = null;
    this.appliedPhase = null;
  }
}

export type CompanionRectangle = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
};

export function placeBubbleNearModel(
  container: { width: number; height: number },
  bubble: { width: number; height: number },
  model: CompanionRectangle,
  padding = 8,
  gap = 10,
): { left: number; top: number; side: "above" | "below" } {
  const maximumLeft = Math.max(padding, container.width - bubble.width - padding);
  const maximumTop = Math.max(padding, container.height - bubble.height - padding);
  const left = Math.min(
    maximumLeft,
    Math.max(padding, model.left + model.width / 2 - bubble.width / 2),
  );
  const above = model.top - bubble.height - gap;
  if (above >= padding) return { left, top: Math.min(above, maximumTop), side: "above" };
  return {
    left,
    top: Math.min(maximumTop, Math.max(padding, model.bottom + gap)),
    side: "below",
  };
}
