import { convertFileSrc } from "@tauri-apps/api/core";

import type { CompanionPhase, CompanionSettings } from "./types";
import {
  CompanionPhaseGate,
  selectLive2DInteractionMotion,
  startLive2DOneShot,
} from "./interaction";
import { encodeSpineAssetUrl } from "./vendor/asset-url.js";
import { Application, Ticker } from "./vendor/pixi-runtime.js";
import { SpinePlayer } from "./vendor/spine-player.js";

export interface CompanionRenderer {
  init(): Promise<void>;
  applyPhase(phase: CompanionPhase): void;
  playInteraction(): boolean;
  containsPoint(x: number, y: number): boolean;
  getInteractiveBounds(): CompanionBounds | null;
  captureFrame?(): string | null;
  destroy(): void;
}

export type CompanionBounds = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
};

export class Spine38Renderer implements CompanionRenderer {
  private readonly player: SpinePlayer;
  private readonly phases = new CompanionPhaseGate();

  constructor(stage: HTMLElement, settings: CompanionSettings) {
    if (!settings.modelPath) {
      throw new Error("Spine model path is required");
    }
    this.player = new SpinePlayer(stage, {
      server: { origin: "" },
      spine: {
        assetUrl: convertFileSrc(settings.modelPath),
        assetDirConfigured: true,
        atlasUrl: settings.atlasPath ? encodeSpineAssetUrl(convertFileSrc(settings.atlasPath)) : undefined,
        skel: settings.modelPath.split(/[\\/]/).pop() ?? "model.skel",
        scale: settings.scale,
        offsetX: 0,
        offsetY: 0,
        mixDurationMs: 420,
        framePadding: 1.04,
        maxViewportFill: 0.86,
        stageBottomInset: 0,
        fitStates: [
          "idle",
          "working",
          "running",
          "waiting",
          "reviewing",
          "success",
        ],
      },
      ui: {
        hudVisible: false,
        dragMode: "compatible",
        frameRateMode: "display",
        maxDevicePixelRatio: 2,
        hitboxPadding: 8,
      },
    });
  }

  async init(): Promise<void> {
    await this.player.init();
  }

  applyPhase(phase: CompanionPhase): void {
    if (!this.phases.requestPhase(phase)) return;
    this.player.applyState(
      { state: phase, source: "lunascope-native" },
      true,
    );
  }

  playInteraction(): boolean {
    const version = this.phases.beginInteraction();
    const entry = this.player.playOneShot(
      { state: "reminder", source: "click" },
      () => {
        const phase = this.phases.finishInteraction(version);
        if (!phase) return;
        this.player.applyState(
          { state: phase, source: "lunascope-native" },
          true,
        );
      },
    );
    if (entry === null) this.phases.cancelInteraction(version);
    return entry !== null;
  }

  containsPoint(x: number, y: number): boolean {
    const bounds = this.getInteractiveBounds();
    return Boolean(
      bounds &&
      x >= bounds.left &&
      x <= bounds.right &&
      y >= bounds.top &&
      y <= bounds.bottom
    );
  }

  getInteractiveBounds(): CompanionBounds | null {
    return this.player.getInteractiveBounds();
  }

  captureFrame(): string | null {
    return this.player.captureFrame();
  }

  destroy(): void {
    this.phases.reset();
    this.player.destroy();
  }
}

declare global {
  interface Window {
    Live2DCubismCore?: unknown;
  }
}

let live2dCorePromise: Promise<void> | null = null;

function loadLive2DCore(path: string): Promise<void> {
  if (window.Live2DCubismCore) return Promise.resolve();
  if (live2dCorePromise) return live2dCorePromise;
  live2dCorePromise = new Promise<void>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = convertFileSrc(path);
    script.async = true;
    script.onload = () => {
      if (window.Live2DCubismCore) resolve();
      else reject(new Error("Cubism Core did not expose Live2DCubismCore"));
    };
    script.onerror = () => reject(new Error("Unable to load Cubism Core"));
    document.head.appendChild(script);
  }).catch((error) => {
    live2dCorePromise = null;
    throw error;
  });
  return live2dCorePromise;
}

const motionCandidates: Record<CompanionPhase, string[]> = {
  idle: ["Idle"],
  working: ["Thinking", "Think", "Work", "Idle"],
  reviewing: ["Review", "Thinking", "TapBody", "Idle"],
  running: ["Running", "Run", "TapBody", "Idle"],
  success: ["Happy", "Success", "TapBody", "Idle"],
  failed: ["Sad", "Failed", "Fail", "Idle"],
  waiting: ["Waiting", "Wait", "Sleep", "Idle"],
};

export class Live2DRenderer implements CompanionRenderer {
  private app: Application | null = null;
  private model: import("./vendor/live2d/cubism4.es.js").Live2DModel | null = null;
  private pendingPhase: CompanionPhase = "idle";
  private manifestUrl: string | null = null;
  private readonly phases = new CompanionPhaseGate();

  constructor(
    private readonly stage: HTMLElement,
    private readonly settings: CompanionSettings,
  ) {
    if (!settings.modelPath || !settings.live2dCorePath) {
      throw new Error("Live2D model and Cubism Core paths are required");
    }
  }

  async init(): Promise<void> {
    await loadLive2DCore(this.settings.live2dCorePath!);
    const { Live2DModel, config } = await import(
      "./vendor/live2d/cubism4.es.js"
    );
    Live2DModel.registerTicker(Ticker);
    config.sound = false;
    config.motionSync = false;
    const app = new Application({
      resizeTo: this.stage,
      transparent: true,
      backgroundAlpha: 0,
      antialias: true,
      autoDensity: true,
      resolution: Math.min(window.devicePixelRatio || 1, 2),
    });
    this.app = app;
    this.stage.appendChild(app.view);
    const modelPath = this.settings.modelPath!;
    const separator = Math.max(modelPath.lastIndexOf("\\"), modelPath.lastIndexOf("/"));
    const root = modelPath.slice(0, separator + 1);
    const response = await fetch(convertFileSrc(modelPath));
    if (!response.ok) {
      throw new Error(`Live2D manifest could not be read (${response.status})`);
    }
    const manifest = (await response.json()) as Record<string, unknown>;
    const references = manifest.FileReferences as
      | Record<string, unknown>
      | undefined;
    if (!references) throw new Error("Live2D manifest is missing FileReferences");
    const assetUrl = (relative: string): string =>
      convertFileSrc(`${root}${relative.replaceAll("/", "\\")}`);
    for (const key of ["Moc", "Physics", "Pose", "UserData", "DisplayInfo"]) {
      if (typeof references[key] === "string") {
        references[key] = assetUrl(references[key] as string);
      }
    }
    if (Array.isArray(references.Textures)) {
      references.Textures = references.Textures.map((value) =>
        typeof value === "string" ? assetUrl(value) : value,
      );
    }
    if (Array.isArray(references.Expressions)) {
      for (const expression of references.Expressions) {
        if (
          expression &&
          typeof expression === "object" &&
          typeof (expression as Record<string, unknown>).File === "string"
        ) {
          const item = expression as Record<string, unknown>;
          item.File = assetUrl(item.File as string);
        }
      }
    }
    if (references.Motions && typeof references.Motions === "object") {
      for (const motions of Object.values(
        references.Motions as Record<string, unknown>,
      )) {
        if (!Array.isArray(motions)) continue;
        for (const motion of motions) {
          if (
            motion &&
            typeof motion === "object" &&
            typeof (motion as Record<string, unknown>).File === "string"
          ) {
            const item = motion as Record<string, unknown>;
            item.File = assetUrl(item.File as string);
            delete item.Sound;
          }
        }
      }
    }
    this.manifestUrl = URL.createObjectURL(
      new Blob([JSON.stringify(manifest)], { type: "application/json" }),
    );
    const model = await Live2DModel.from(this.manifestUrl, {
      autoInteract: false,
      autoUpdate: true,
    });
    model.anchor.set(0.5, 1);
    const width = app.renderer.width / app.renderer.resolution;
    const height = app.renderer.height / app.renderer.resolution;
    const fit = Math.min(width / model.width, height / model.height) * 0.94;
    model.scale.set(fit * this.settings.scale);
    model.position.set(width / 2, height);
    app.stage.addChild(model);
    this.model = model;
    this.applyPhase(this.pendingPhase);
  }

  applyPhase(phase: CompanionPhase): void {
    this.pendingPhase = phase;
    if (!this.phases.requestPhase(phase)) return;
    this.playPhase(phase);
  }

  private playPhase(phase: CompanionPhase): void {
    if (!this.model) return;
    const definitions =
      this.model.internalModel.motionManager?.definitions ?? {};
    const groups = Object.keys(definitions);
    const group = motionCandidates[phase]
      .map((candidate) =>
        groups.find((value) => value.toLowerCase() === candidate.toLowerCase()),
      )
      .find(Boolean);
    if (group) {
      void this.model.motion(group, undefined, 3).catch(() => {
        // Model-specific motion failures fall back to its own idle controller.
      });
    }
  }

  playInteraction(): boolean {
    const model = this.model;
    if (!model) return false;
    const manager = model.internalModel.motionManager;
    if (!manager) return false;
    const group = selectLive2DInteractionMotion(Object.keys(manager?.definitions ?? {}));
    if (!group) return false;

    const version = this.phases.beginInteraction();
    const restorePhase = () => {
      const phase = this.phases.finishInteraction(version);
      if (phase) this.playPhase(phase);
    };
    void startLive2DOneShot(
      () => model.motion(group, undefined, 3),
      () => manager.once("motionFinish", restorePhase),
      restorePhase,
    );
    return true;
  }

  containsPoint(x: number, y: number): boolean {
    if (!this.model) return false;
    return this.model.hitTest(x, y).length > 0 || this.model.containsPoint({ x, y });
  }

  getInteractiveBounds(): CompanionBounds | null {
    if (!this.model) return null;
    const bounds = this.model.getBounds(true);
    return {
      left: bounds.x,
      right: bounds.x + bounds.width,
      top: bounds.y,
      bottom: bounds.y + bounds.height,
      width: bounds.width,
      height: bounds.height,
    };
  }

  destroy(): void {
    this.phases.reset();
    this.model?.destroy({ children: true });
    this.model = null;
    this.app?.destroy(true, { children: true, texture: true, baseTexture: true });
    this.app = null;
    if (this.manifestUrl) URL.revokeObjectURL(this.manifestUrl);
    this.manifestUrl = null;
  }
}

export function companionRenderer(
  stage: HTMLElement,
  settings: CompanionSettings,
): CompanionRenderer {
  return settings.modelKind === "live2d"
    ? new Live2DRenderer(stage, settings)
    : new Spine38Renderer(stage, settings);
}
