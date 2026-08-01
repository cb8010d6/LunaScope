import * as PIXI from "pixi.js";
import { Spine } from "pixi-spine";
import { animationForState, stateMachine } from "./state.js";
import { spineAssetUrl } from "./asset-url.js";
import { calculateInteractiveBounds, compactPointerRegions, expandBounds, normalizePointerRegions, transformLocalBounds, unionPointerRegions } from "./hitbox.js";
import { acquireSpineAsset } from "./spine-asset-handle.js";

const ACTIVE_BOUNDS_STATES = new Set(["working", "running", "reviewing", "success", "reminder", "failed"]);
const TRACK_STALE_MS = 3000;
const TRACK_REBUILD_DELAY_MS = 2000;
const RECOVERY_COOLDOWN_MS = 30000;
const RECOVERY_WINDOW_MS = 5 * 60 * 1000;
const MAX_RECOVERIES_PER_WINDOW = 3;

export function activeRecoveryHistory(history = [], now = Date.now()) {
  return history.filter((at) => Number.isFinite(at) && now - at <= RECOVERY_WINDOW_MS);
}

export class ReplacementRevision {
  constructor() {
    this.current = 0;
  }

  begin() {
    this.current += 1;
    return this.current;
  }

  isCurrent(revision) {
    return revision === this.current;
  }

  invalidate() {
    this.current += 1;
  }
}

export function modelCoreFitStates(spineConfig = {}) {
  const configured = spineConfig.coreFitStates;
  return Array.isArray(configured) && configured.length
    ? configured
    : ["idle", "working", "running", "waiting"];
}

export function modelViewportProfile(spineConfig = {}) {
  const fitMode = String(spineConfig.fitMode || "legacy");
  const illustration = fitMode === "character" || (fitMode === "legacy" && (
    spineConfig.modelCategory === "illustration"
    || spineConfig.compatibilityProfile === "idle-only"
  ));
  const configuredPadding = Math.max(1, Number(spineConfig.framePadding || 1.12));
  const configuredFill = Math.max(0.45, Math.min(0.9, Number(spineConfig.maxViewportFill || 0.72)));
  return illustration
    ? {
        framePadding: Math.min(configuredPadding, 1.02),
        availableWidth: 0.92,
        availableHeight: 0.98,
        viewportFill: Math.max(configuredFill, 0.88),
        scaleBoost: 1.15
      }
    : {
        framePadding: configuredPadding,
        availableWidth: 0.86,
        availableHeight: 0.94,
        viewportFill: configuredFill,
        scaleBoost: 1
      };
}

export function attachTrackCompletion(entry, onComplete, isCurrent = () => true) {
  if (!entry) return null;
  entry.listener = {
    complete: () => {
      if (isCurrent()) onComplete?.();
    }
  };
  return entry;
}

export function selectAvailableAnimation(animationNames = [], requested = "") {
  const names = animationNames.filter((name) => typeof name === "string" && name);
  const requestedKey = String(requested || "").toLowerCase();
  const exact = names.find((name) => name.toLowerCase() === requestedKey);
  if (exact) return exact;

  const stable = ["idle", "default", "relax", "stand"];
  const fallbacks = {
    relax: ["relax", ...stable],
    idle: stable,
    default: ["default", "idle", "relax", "stand"],
    move: ["move", "run", "walk", ...stable],
    interact: ["interact", "touch", "tap", ...stable],
    sit: ["sit", ...stable],
    sleep: ["sleep", ...stable],
    special: ["special", "interact", ...stable]
  };
  const semantics = fallbacks[requestedKey] || [requestedKey, ...stable].filter(Boolean);
  const matchesSemantic = (name, semantic) => String(name).toLowerCase()
    .split(/[^a-z0-9]+/)
    .some((part) => part === semantic || (part.startsWith(semantic) && /^\d+$/.test(part.slice(semantic.length))));
  for (const semantic of semantics) {
    const match = names.find((name) => matchesSemantic(name, semantic));
    if (match) return match;
  }
  return "";
}

export function normalizeFrameRateMode(value) {
  const mode = String(value || "display");
  return mode === "60" || mode === "30" ? mode : "display";
}

export function tickerMaxFps(frameRateMode, dragActive = false, dragMode = "smooth") {
  const mode = normalizeFrameRateMode(frameRateMode);
  const selected = mode === "60" ? 60 : mode === "30" ? 30 : 0;
  if (dragActive && dragMode === "compatible") return selected > 0 ? Math.min(42, selected) : 42;
  return selected;
}

export function trackEntryNeedsReplay(entry, lastProgressAt = 0, now = Date.now()) {
  if (!entry) return true;
  if (entry.loop) {
    return Number(entry.timeScale ?? 1) > 0
      && lastProgressAt > 0
      && now - lastProgressAt > TRACK_STALE_MS;
  }
  return typeof entry.isComplete === "function" ? entry.isComplete() : false;
}

export function animationRecoveryAction({
  step = 0,
  stale = false,
  progressed = false,
  elapsedMs = 0,
  previousRecoveryAgeMs = Infinity,
  recentRecoveries = 0
} = {}) {
  if (progressed) return "none";
  if (step === 0) {
    if (!stale || previousRecoveryAgeMs < RECOVERY_COOLDOWN_MS) return "none";
    return recentRecoveries >= MAX_RECOVERIES_PER_WINDOW ? "rate-limited" : "replay";
  }
  if (elapsedMs < TRACK_REBUILD_DELAY_MS) return "none";
  return step === 1 ? "rebuild" : "recreate";
}

export class SpinePlayer {
  constructor(stage, config) {
    this.stageElement = stage;
    this.config = config;
    this.app = null;
    this.model = null;
    this.spine = null;
    this.currentKey = "";
    this.currentMotionKey = "";
    this.baseScale = 1;
    this.userScale = 1;
    this.direction = "right";
    this.hudVisible = config.ui?.hudVisible !== false;
    this.returnTimer = null;
    this.stableBounds = null;
    this.fitBounds = null;
    this.screenScale = 1;
    this.anchor = { x: 20, y: 28, scale: 1 };
    this.anchorTarget = null;
    this.dragMode = config.ui?.dragMode || "smooth";
    this.frameRateMode = normalizeFrameRateMode(config.ui?.frameRateMode);
    this.dragActive = false;
    this.pointerNear = false;
    this.minUserScale = 0.35;
    this.maxUserScale = 1.55;
    this.minPresentationScale = 0.2;
    this.maxPresentationScale = 2.5;
    this.baseFitScale = null;
    this.lastLayoutSize = { width: 0, height: 0 };
    this.resizeTimer = null;
    this.gpuRecoveryTimer = null;
    this.gpuRecoveryRequested = false;
    this.handleResize = null;
    this.handleWheel = null;
    this.handleContextLost = null;
    this.handleContextRestored = null;
    this.lastFrameAt = 0;
    this.frameCounter = 0;
    this.resetDeltaOnNextTick = true;
    this.resumeGraceUntil = 0;
    this.lastTrackSampleAt = 0;
    this.lastTrackTime = -1;
    this.lastTrackProgressAt = 0;
    this.trackSnapshot = null;
    this.animationRecoveryStep = 0;
    this.animationRecoveryAt = 0;
    this.animationRecoveryHistory = [];
    this.animationRecoveryExhausted = false;
    this.replacementRevision = new ReplacementRevision();
    this.currentStateInput = { state: "idle", source: "system" };
    this.lastBoundsEmitAt = 0;
    this.lastEmittedBounds = null;
    this.onInteractiveBoundsChange = null;
    this.frameTicker = null;
    this.handleVisibilityChange = null;
    this.runtimeBoundsOffset = { x: 0, y: 0 };
    this.runtimeBoundsSize = { x: 0, y: 0 };
    this.runtimeBoundsScratch = [];
    this.hitboxPadding = Number.isFinite(Number(config.ui?.hitboxPadding))
      ? Math.min(48, Math.max(0, Number(config.ui.hitboxPadding)))
      : 8;
  }

  async init() {
    const dprLimit = Number.isFinite(Number(this.config.ui?.maxDevicePixelRatio))
      ? Math.min(3, Math.max(1, Number(this.config.ui.maxDevicePixelRatio)))
      : 2;
    this.app = new PIXI.Application({
      resizeTo: this.stageElement,
      transparent: true,
      backgroundAlpha: 0,
      backgroundColor: 0x000000,
      antialias: true,
      autoDensity: true,
      clearBeforeRender: true,
      powerPreference: "default",
      resolution: Math.min(window.devicePixelRatio || 1, dprLimit)
    });
    this.stageElement.appendChild(this.app.view);
    this.disablePixiInteraction();
    this.frameTicker = () => this.tick(this.app?.ticker?.deltaMS);
    this.app.ticker.add(this.frameTicker, undefined, PIXI.UPDATE_PRIORITY.HIGH);
    this.applyTickerMode();
    this.configureTransparentRenderer();
    this.bindContextRecovery();
    this.model = new PIXI.Container();
    this.app.stage.addChild(this.model);

    await this.loadSpine();
    this.handleResize = () => {
      window.clearTimeout(this.resizeTimer);
      this.resizeTimer = window.setTimeout(() => this.layout({ forceFitRecalc: true }), 90);
    };
    this.handleWheel = (event) => {
      event.preventDefault();
      this.adjustPresentationScale(event.deltaY > 0 ? -0.05 : 0.05);
    };
    this.handleVisibilityChange = () => {
      this.resetDeltaOnNextTick = true;
      this.resumeGraceUntil = Date.now() + 5000;
      this.lastTrackProgressAt = Date.now();
    };
    window.addEventListener("resize", this.handleResize);
    document.addEventListener("visibilitychange", this.handleVisibilityChange);
    this.stageElement.addEventListener("wheel", this.handleWheel, { passive: false });
  }

  disablePixiInteraction() {
    const interaction = this.app?.renderer?.plugins?.interaction;
    if (!interaction) return;
    interaction.useSystemTicker = false;
    interaction.removeEvents?.();
    this.app.stage.interactive = false;
    this.app.stage.interactiveChildren = false;
  }

  tick(deltaMs) {
    const now = Date.now();
    this.lastFrameAt = now;
    this.frameCounter += 1;
    let safeDeltaMs = Number(deltaMs || 0);
    if (this.resetDeltaOnNextTick || document.visibilityState !== "visible") {
      safeDeltaMs = 0;
      this.resetDeltaOnNextTick = false;
    }
    safeDeltaMs = Math.max(0, Math.min(100, safeDeltaMs));
    if (this.spine && safeDeltaMs > 0) {
      this.spine.update(safeDeltaMs / 1000);
    }

    if (now - this.lastTrackSampleAt >= 250) {
      this.lastTrackSampleAt = now;
      this.sampleTrackProgress(now);
      this.recoverStalledAnimation(now);
    }

    const stateId = this.currentStateInput?.state || "idle";
    const boundsInterval = this.pointerNear || this.dragActive || ACTIVE_BOUNDS_STATES.has(stateId) ? 33 : 100;
    if (now - this.lastBoundsEmitAt >= boundsInterval) {
      this.lastBoundsEmitAt = now;
      this.emitInteractiveBounds();
    }
  }

  sampleTrackProgress(now = Date.now()) {
    const entry = this.spine?.state?.getCurrent?.(0) || null;
    const animationName = String(entry?.animation?.name || "");
    const trackTime = Number(entry?.trackTime ?? -1);
    const changedTrack = animationName !== this.trackSnapshot?.animationName;
    const progressed = changedTrack
      || trackTime < this.lastTrackTime
      || Math.abs(trackTime - this.lastTrackTime) >= 0.001;
    if (progressed) {
      this.lastTrackProgressAt = now;
      if (this.animationRecoveryStep > 0) {
        this.animationRecoveryStep = 0;
        this.animationRecoveryAt = 0;
      }
    }
    this.lastTrackTime = trackTime;
    this.trackSnapshot = entry ? {
      animationName,
      trackTime,
      animationEnd: Number(entry.animationEnd ?? entry.animation?.duration ?? 0),
      loop: Boolean(entry.loop),
      timeScale: Number(entry.timeScale ?? 1)
    } : null;
  }

  resetTrackProgressBaseline(now = Date.now()) {
    const entry = this.spine?.state?.getCurrent?.(0) || null;
    this.lastTrackTime = Number(entry?.trackTime ?? -1);
    this.lastTrackProgressAt = now;
    this.trackSnapshot = entry ? {
      animationName: String(entry.animation?.name || ""),
      trackTime: this.lastTrackTime,
      animationEnd: Number(entry.animationEnd ?? entry.animation?.duration ?? 0),
      loop: Boolean(entry.loop),
      timeScale: Number(entry.timeScale ?? 1)
    } : null;
  }

  emitInteractiveBounds(force = false) {
    const regions = this.getInteractiveRegions();
    const previous = this.lastEmittedRegions || [];
    const changed = force || regions.length !== previous.length
      || regions.some((bounds, index) => ["left", "right", "top", "bottom"]
        .some((key) => Math.abs(bounds[key] - previous[index]?.[key]) >= 1));
    if (!changed) return;
    this.lastEmittedRegions = regions.map((bounds) => ({ ...bounds }));
    this.onInteractiveBoundsChange?.(this.lastEmittedRegions);
  }

  setPointerProximity(near) {
    this.pointerNear = Boolean(near);
    if (this.pointerNear) this.emitInteractiveBounds(true);
  }

  configureTransparentRenderer() {
    if (!this.app?.renderer) return;
    this.app.view.style.background = "transparent";
    try {
      if (this.app.renderer.background) {
        this.app.renderer.background.color = 0x000000;
        this.app.renderer.background.alpha = 0;
      }
      if ("backgroundAlpha" in this.app.renderer) {
        this.app.renderer.backgroundAlpha = 0;
      }
      if ("clearBeforeRender" in this.app.renderer) {
        this.app.renderer.clearBeforeRender = true;
      }
    } catch (error) {
      console.warn("[spine-companion] unable to enforce transparent renderer", error);
    }
  }

  bindContextRecovery() {
    const view = this.app?.view;
    if (!view) return;
    this.handleContextLost = (event) => {
      event.preventDefault();
      this.app.view.style.visibility = "hidden";
      this.onGpuContextLost?.({ reason: "webglcontextlost" });
      window.clearTimeout(this.gpuRecoveryTimer);
      this.gpuRecoveryTimer = window.setTimeout(() => {
        this.requestGpuRecovery("webglcontextlost-timeout");
      }, 800);
    };
    this.handleContextRestored = () => {
      this.configureTransparentRenderer();
      this.requestGpuRecovery("webglcontextrestored");
    };
    view.addEventListener("webglcontextlost", this.handleContextLost, false);
    view.addEventListener("webglcontextrestored", this.handleContextRestored, false);
  }

  requestGpuRecovery(reason, recreateWindow = false) {
    if (this.gpuRecoveryRequested) return;
    this.gpuRecoveryRequested = true;
    this.onGpuRecoveryRequested?.({ reason, recreateWindow });
  }

  async loadSpine() {
    if (!this.config.spine.assetDirConfigured) {
      throw new Error("No Spine asset directory is configured.");
    }

    const { resource, handle, identity } = await this.loadSpineResourceWithRetry(this.config);
    this.assetHandle = handle;
    this.assetIdentity = identity;
    try {
      this.spine = new Spine(resource.spineData);
      this.configureSpineInstance(this.spine);
      this.model.addChild(this.spine);

      this.stableBounds = this.measureStableBounds(stateMachine.states);
      const coreFitStates = modelCoreFitStates(this.config.spine);
      this.fitBounds = this.measureStableBounds(coreFitStates)
        || this.measureStableBounds(this.config.spine.fitStates || stateMachine.states);
      this.applyState({ state: "idle", source: "system" }, true);
    } catch (error) {
      this.spine?.destroy?.({ children: true, texture: false, baseTexture: false });
      this.spine = null;
      this.assetHandle.release();
      this.assetHandle = null;
      this.assetIdentity = "";
      throw error;
    }
  }

  configureSpineInstance(spine, config = this.config) {
    spine.autoUpdate = false;
    spine.localDelayLimit = 0.1;
    spine.stateData.defaultMix = Number(config.spine.mixDurationMs || 280) / 1000;
  }

  async loadSpineResourceWithRetry(config = this.config) {
    const attempts = 3;
    let lastError = null;
    const identity = spineAssetUrl(config);
    for (let attempt = 1; attempt <= attempts; attempt += 1) {
      const handle = acquireSpineAsset(identity, { resourceName: "companion" });
      try {
        const resource = await handle.load();
        return { resource, handle, identity };
      } catch (error) {
        handle.release();
        lastError = error;
        if (attempt < attempts) {
          await new Promise((resolve) => window.setTimeout(resolve, attempt * 350));
        }
      }
    }
    throw lastError;
  }

  async replaceConfig(nextConfig, state = this.currentStateInput) {
    const replacementRevision = this.replacementRevision.begin();
    if (!nextConfig?.spine?.assetDirConfigured) {
      throw new Error("No Spine asset directory is configured.");
    }
    const nextIdentity = spineAssetUrl(nextConfig);
    if (nextIdentity === this.assetIdentity && this.spine) {
      const previousBoundsSignature = JSON.stringify({
        fitStates: this.config.spine.fitStates,
        coreFitStates: this.config.spine.coreFitStates,
        boundsSamples: this.config.spine.boundsSamples
      });
      const nextBoundsSignature = JSON.stringify({
        fitStates: nextConfig.spine.fitStates,
        coreFitStates: nextConfig.spine.coreFitStates,
        boundsSamples: nextConfig.spine.boundsSamples
      });
      this.config = nextConfig;
      this.userScale = 1;
      this.configureSpineInstance(this.spine, nextConfig);
      this.frameRateMode = normalizeFrameRateMode(nextConfig.ui?.frameRateMode);
      this.applyTickerMode();
      if (previousBoundsSignature !== nextBoundsSignature) {
        this.stableBounds = this.measureStableBounds(stateMachine.states);
        this.fitBounds = this.measureStableBounds(modelCoreFitStates(nextConfig.spine))
          || this.measureStableBounds(nextConfig.spine.fitStates || stateMachine.states);
        this.applyState(state, true);
      }
      this.baseFitScale = null;
      this.layout({ forceFitRecalc: true });
      this.emitInteractiveBounds(true);
      return true;
    }

    const loaded = await this.loadSpineResourceWithRetry(nextConfig);
    if (!this.replacementRevision.isCurrent(replacementRevision)) {
      loaded.handle.release();
      return false;
    }
    const previous = {
      config: this.config,
      spine: this.spine,
      handle: this.assetHandle,
      identity: this.assetIdentity,
      stableBounds: this.stableBounds,
      fitBounds: this.fitBounds,
      currentKey: this.currentKey,
      currentMotionKey: this.currentMotionKey,
      currentStateInput: this.currentStateInput
    };
    let replacement = null;
    try {
      replacement = new Spine(loaded.resource.spineData);
      this.configureSpineInstance(replacement, nextConfig);
      this.model.addChild(replacement);
      this.config = nextConfig;
      this.spine = replacement;
      this.assetHandle = loaded.handle;
      this.assetIdentity = loaded.identity;
      this.currentKey = "";
      this.currentMotionKey = "";
      this.stableBounds = this.measureStableBounds(stateMachine.states);
      this.fitBounds = this.measureStableBounds(modelCoreFitStates(nextConfig.spine))
        || this.measureStableBounds(nextConfig.spine.fitStates || stateMachine.states);
      this.baseFitScale = null;
      this.applyState(state, true);
      this.layout({ forceFitRecalc: true });
      this.model.removeChild(previous.spine);
      previous.spine?.destroy?.({ children: true, texture: false, baseTexture: false });
      previous.handle?.release();
      this.animationRecoveryHistory = [];
      this.animationRecoveryExhausted = false;
      this.emitInteractiveBounds(true);
      return true;
    } catch (error) {
      if (replacement?.parent === this.model) this.model.removeChild(replacement);
      replacement?.destroy?.({ children: true, texture: false, baseTexture: false });
      loaded.handle.release();
      this.config = previous.config;
      this.spine = previous.spine;
      this.assetHandle = previous.handle;
      this.assetIdentity = previous.identity;
      this.stableBounds = previous.stableBounds;
      this.fitBounds = previous.fitBounds;
      this.currentKey = previous.currentKey;
      this.currentMotionKey = previous.currentMotionKey;
      this.currentStateInput = previous.currentStateInput;
      this.layout();
      throw error;
    }
  }

  setUserScale(scale) {
    this.setPresentationScale(scale);
  }

  setPresentation(presentation = {}, notify = false) {
    if (!this.config.spine) this.config.spine = {};
    const scale = Number(presentation.scale ?? this.config.spine.scale ?? 1);
    const offsetX = Number(presentation.offsetX ?? this.config.spine.offsetX ?? 0);
    const offsetY = Number(presentation.offsetY ?? this.config.spine.offsetY ?? 0);
    this.config.spine.scale = Number.isFinite(scale)
      ? Math.min(this.maxPresentationScale, Math.max(this.minPresentationScale, scale))
      : 1;
    this.config.spine.offsetX = Number.isFinite(offsetX) ? offsetX : 0;
    this.config.spine.offsetY = Number.isFinite(offsetY) ? offsetY : 0;
    this.config.spine.fitMode = ["legacy", "character", "full"].includes(presentation.fitMode)
      ? presentation.fitMode
      : String(this.config.spine.fitMode || "legacy");
    this.userScale = 1;
    this.baseFitScale = null;
    this.layout();
    this.emitInteractiveBounds(true);
    if (notify) this.onPresentationChange?.(this.getPresentation());
  }

  getPresentation() {
    return {
      scale: Number(this.config.spine?.scale || 1),
      offsetX: Number(this.config.spine?.offsetX || 0),
      offsetY: Number(this.config.spine?.offsetY || 0),
      fitMode: String(this.config.spine?.fitMode || "legacy")
    };
  }

  setPresentationScale(scale, notify = true) {
    this.setPresentation({ ...this.getPresentation(), scale }, notify);
  }

  adjustPresentationScale(delta) {
    const nextDelta = Number(delta);
    if (!Number.isFinite(nextDelta)) return;
    this.setPresentationScale(Number(this.config.spine?.scale || 1) + nextDelta);
  }

  adjustUserScale(delta) {
    this.adjustPresentationScale(delta);
  }

  resetUserScale() {
    const defaultScale = Number(this.config.spine?.presentationDefaults?.scale || 0.86);
    this.setPresentationScale(defaultScale);
  }

  setHudVisible(visible) {
    this.hudVisible = Boolean(visible);
    this.layout();
  }

  setDragMode(mode) {
    this.dragMode = mode === "smooth" ? "smooth" : "compatible";
    this.applyTickerMode();
  }

  setFrameRateMode(mode) {
    this.frameRateMode = normalizeFrameRateMode(mode);
    this.applyTickerMode();
  }

  setHitboxPadding(value) {
    this.hitboxPadding = Number.isFinite(Number(value))
      ? Math.min(48, Math.max(0, Number(value)))
      : 8;
    this.emitInteractiveBounds(true);
  }

  setDragActive(active) {
    this.dragActive = Boolean(active);
    this.applyTickerMode();
    if (!this.dragActive) this.emitInteractiveBounds(true);
  }

  applyTickerMode() {
    if (!this.app) return;
    this.app.ticker.maxFPS = tickerMaxFps(this.frameRateMode, this.dragActive, this.dragMode);
  }

  setDirection(direction) {
    this.direction = direction === "left" ? "left" : "right";
    this.layout();
  }

  applyState(state, force = false) {
    if (!this.spine) return;
    this.currentStateInput = { ...state };

    let motion = animationForState(state, this.config);
    if (motion.state === "running") {
      motion = state?.source === "drag"
        ? { ...motion, animation: "Move", loop: true, segment: null, tailSegment: null, repeatSegment: false }
        : { ...motion, animation: "Relax", loop: true, segment: null, tailSegment: null, repeatSegment: false };
    }
    const animationNames = this.spine.spineData.animations.map((animation) => animation.name);
    const resolvedAnimation = selectAvailableAnimation(animationNames, motion.animation);
    if (!resolvedAnimation) return;
    motion = resolvedAnimation === motion.animation
      ? motion
      : {
          ...motion,
          animation: resolvedAnimation,
          loop: true,
          segment: null,
          tailSegment: null,
          repeatSegment: false,
          returnTo: null,
          returnAfterMs: null
        };
    const segment = motion.segment ? this.config.specialSegments?.[motion.segment] : null;
    const tailSegment = motion.tailSegment ? this.config.specialSegments?.[motion.tailSegment] : null;
    const nextDirection = motion.state === "running" ? (state.direction || "right") : "right";
    const segmentKey = segment ? `${segment.from || 0}-${segment.to || "end"}` : "full";
    const tailKey = tailSegment ? `>${tailSegment.from || 0}-${tailSegment.to || "end"}:${tailSegment.loop ?? true}` : "";
    const motionKey = `${motion.animation}:${segmentKey}:${segment?.loop ?? motion.loop ?? true}${tailKey}`;
    const key = `${motionKey}:${nextDirection}`;

    this.direction = nextDirection;
    if (!force && key === this.currentKey) {
      const entry = this.spine.state.getCurrent(0);
      if (!trackEntryNeedsReplay(entry, this.lastTrackProgressAt)) return;
      force = true;
    }
    if (!force && motionKey === this.currentMotionKey) {
      this.currentKey = key;
      this.layout();
      return;
    }
    this.currentKey = key;
    this.currentMotionKey = motionKey;

    window.clearTimeout(this.returnTimer);
    const entry = this.spine.state.setAnimation(0, motion.animation, segment?.loop ?? motion.loop ?? true);
    entry.mixDuration = Number(this.config.spine.mixDurationMs || 280) / 1000;

    if (segment) {
      entry.animationStart = Number(segment.from || 0);
      entry.animationEnd = Number(segment.to || entry.animation.duration);
      entry.trackTime = 0;
    }

    if (tailSegment && !(segment?.loop ?? motion.loop ?? true)) {
      const tailShouldRepeat = tailSegment.loop !== false;
      const tailMixDuration = Number(tailSegment.mixDurationMs || this.config.spine.mixDurationMs || 280) / 1000;
      const tailEntry = this.spine.state.addAnimation(0, motion.animation, tailShouldRepeat, 0);
      tailEntry.mixDuration = tailMixDuration;
      tailEntry.animationStart = Number(tailSegment.from || 0);
      tailEntry.animationEnd = Number(tailSegment.to || tailEntry.animation.duration);
      tailEntry.trackTime = 0;
    }

    if (motion.repeatSegment && segment && !(segment.loop ?? motion.loop ?? true)) {
      const mixDuration = Number(motion.repeatMixDurationMs || this.config.spine.mixDurationMs || 280) / 1000;
      const repeatEntry = this.spine.state.addAnimation(0, motion.animation, true, 0);
      repeatEntry.mixDuration = mixDuration;
      repeatEntry.animationStart = Number(segment.from || 0);
      repeatEntry.animationEnd = Number(segment.to || repeatEntry.animation.duration);
      repeatEntry.trackTime = 0;
    }

    if (motion.returnTo && motion.returnAfterMs && !force) {
      this.returnTimer = window.setTimeout(() => {
        this.onAutoReturn?.(motion.returnTo);
      }, Number(motion.returnAfterMs));
    }

    this.applyStableAnchor();
    this.layout();
    this.resetTrackProgressBaseline();
    return entry;
  }

  stateDurationMs(state = {}) {
    if (!this.spine) return 0;
    const motion = animationForState(state, this.config);
    const animation = this.spine.spineData.animations.find((item) => item.name === motion.animation);
    if (!animation) return 0;
    const segment = motion.segment ? this.config.specialSegments?.[motion.segment] : null;
    const from = Number(segment?.from ?? 0);
    const to = Number(segment?.to ?? animation.duration ?? 0);
    return Math.max(0, to - from) * 1000;
  }

  playOneShot(state = {}, onComplete) {
    const entry = this.applyState(state, true);
    if (!entry) return null;
    const token = Symbol("one-shot");
    this.oneShotToken = token;
    attachTrackCompletion(entry, () => {
      this.oneShotToken = null;
      onComplete?.();
    }, () => this.oneShotToken === token);
    return entry;
  }

  recoverStalledAnimation(now = Date.now()) {
    this.animationRecoveryHistory = activeRecoveryHistory(this.animationRecoveryHistory, now);
    if (this.animationRecoveryExhausted && this.animationRecoveryHistory.length < MAX_RECOVERIES_PER_WINDOW) {
      this.animationRecoveryExhausted = false;
    }
    if (document.visibilityState !== "visible" || now < this.resumeGraceUntil || this.animationRecoveryExhausted) {
      return false;
    }
    const entry = this.spine?.state?.getCurrent?.(0);
    if (!entry?.loop || Number(entry.timeScale ?? 1) <= 0) return false;

    const previousRecovery = this.animationRecoveryHistory.at(-1) || 0;
    const progressedAfterRecovery = this.animationRecoveryStep > 0
      && this.lastTrackProgressAt > this.animationRecoveryAt;
    const action = animationRecoveryAction({
      step: this.animationRecoveryStep,
      stale: trackEntryNeedsReplay(entry, this.lastTrackProgressAt, now),
      progressed: progressedAfterRecovery,
      elapsedMs: now - this.animationRecoveryAt,
      previousRecoveryAgeMs: previousRecovery ? now - previousRecovery : Infinity,
      recentRecoveries: this.animationRecoveryHistory.length
    });
    if (action === "none") return false;
    if (action === "rate-limited") {
        this.animationRecoveryExhausted = true;
        this.onHealthWarning?.({ reason: "animation-recovery-rate-limited" });
        return false;
    }
    if (action === "replay") {
      this.animationRecoveryHistory.push(now);
      this.applyState(this.currentStateInput, true);
      this.animationRecoveryStep = 1;
      this.animationRecoveryAt = this.lastTrackProgressAt;
      return true;
    }
    if (action === "rebuild") {
      try {
        this.rebuildSpineInstance("animation-track-stale");
        this.animationRecoveryStep = 2;
        this.animationRecoveryAt = this.lastTrackProgressAt;
        return true;
      } catch (error) {
        this.requestGpuRecovery("animation-instance-rebuild-failed", true);
        return false;
      }
    }
    if (action === "recreate") {
      this.requestGpuRecovery("animation-track-recovery-failed", true);
    }
    return false;
  }

  rebuildSpineInstance(reason = "renderer-rebuild") {
    if (!this.spine?.spineData || !this.model) throw new Error("Spine data is unavailable for rebuild.");
    const previous = this.spine;
    const replacement = new Spine(previous.spineData);
    this.configureSpineInstance(replacement);
    this.model.addChild(replacement);
    this.spine = replacement;
    this.currentKey = "";
    this.currentMotionKey = "";
    this.applyStableAnchor();
    this.applyState(this.currentStateInput, true);
    this.model.removeChild(previous);
    previous.destroy?.({ children: true, texture: false, baseTexture: false });
    this.lastRecoveryReason = reason;
    this.emitInteractiveBounds(true);
  }

  measureStableBounds(stateIds) {
    if (!this.spine) return;
    const sampleCount = Math.max(3, Number(this.config.spine.boundsSamples || 10));
    const animations = new Map(this.spine.spineData.animations.map((animation) => [animation.name, animation]));
    const measured = [];
    const originalAutoUpdate = this.spine.autoUpdate;
    this.spine.autoUpdate = false;
    this.spine.position.set(0, 0);
    this.spine.scale.set(1, 1);

    for (const stateId of stateIds) {
      const motion = animationForState({ state: stateId }, this.config);
      const resolvedAnimation = selectAvailableAnimation([...animations.keys()], motion.animation);
      const animation = animations.get(resolvedAnimation);
      if (!animation) continue;
      const segment = resolvedAnimation === motion.animation && motion.segment
        ? this.config.specialSegments?.[motion.segment]
        : null;
      const from = Number(segment?.from ?? 0);
      const to = Number(segment?.to ?? animation.duration ?? 1);
      const span = Math.max(0.001, to - from);

      for (let index = 0; index < sampleCount; index += 1) {
        const ratio = sampleCount === 1 ? 0 : index / (sampleCount - 1);
        this.sampleAnimationBounds(resolvedAnimation, from + span * ratio, Boolean(segment?.loop ?? motion.loop ?? true), measured);
      }
    }

    this.spine.autoUpdate = originalAutoUpdate;
    if (measured.length === 0) return null;

    const union = measured.reduce((acc, bounds) => ({
      minX: Math.min(acc.minX, bounds.x),
      minY: Math.min(acc.minY, bounds.y),
      maxX: Math.max(acc.maxX, bounds.x + bounds.width),
      maxY: Math.max(acc.maxY, bounds.y + bounds.height)
    }), { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity });

    return {
      x: union.minX,
      y: union.minY,
      width: union.maxX - union.minX,
      height: union.maxY - union.minY
    };
  }

  sampleAnimationBounds(animation, time, loop, measured) {
    this.spine.skeleton.setToSetupPose();
    this.spine.state.clearTracks();
    this.spine.state.setAnimation(0, animation, loop);
    this.spine.state.update(time);
    this.spine.state.apply(this.spine.skeleton);
    this.spine.skeleton.updateWorldTransform();
    this.spine.update(0);
    const bounds = this.spine.getLocalBounds();
    if (Number.isFinite(bounds.width) && Number.isFinite(bounds.height) && bounds.width > 1 && bounds.height > 1) {
      measured.push({ x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height });
    }
  }

  applyStableAnchor() {
    if (!this.spine) return;
    const bounds = this.fitBounds || this.stableBounds || this.spine.getLocalBounds();
    this.spine.x = -(bounds.x + bounds.width / 2);
    this.spine.y = -(bounds.y + bounds.height);
  }

  layout(options = {}) {
    if (!this.app || !this.spine) return;

    const width = this.stageElement.clientWidth || this.app.renderer.width / this.app.renderer.resolution;
    const height = this.stageElement.clientHeight || this.app.renderer.height / this.app.renderer.resolution;
    if (width < 120 || height < 160) return;
    const fitBounds = this.fitBounds || this.stableBounds || this.spine.getLocalBounds();
    const boundsWidth = Math.max(80, Number(fitBounds.width || 0));
    const boundsHeight = Math.max(120, Number(fitBounds.height || 0));
    const viewport = modelViewportProfile(this.config.spine);
    const padding = viewport.framePadding;
    const availableWidth = Math.max(1, width * viewport.availableWidth);
    const bottomInset = this.hudVisible ? Number(this.config.spine.stageBottomInset || 0) : 0;
    const usableHeight = Math.max(1, height - bottomInset);
    const availableHeight = Math.max(1, usableHeight * viewport.availableHeight);
    const measuredFitScale = Math.min(availableWidth / (boundsWidth * padding), availableHeight / (boundsHeight * padding));
    const sizeChanged = Math.abs(width - this.lastLayoutSize.width) > 8 || Math.abs(height - this.lastLayoutSize.height) > 8;
    if (!this.baseFitScale || options.forceFitRecalc || sizeChanged) {
      this.baseFitScale = measuredFitScale;
      this.lastLayoutSize = { width, height };
    }
    const fitScale = Math.min(measuredFitScale, this.baseFitScale * 1.08);
    const configuredScale = Number(this.config.spine.scale || 1);
    const rawScale = fitScale * configuredScale * this.userScale * viewport.scaleBoost;
    const fill = viewport.viewportFill;
    const maxScaleByHeight = (height * fill) / boundsHeight;
    const maxScaleByWidth = (width * viewport.availableWidth) / boundsWidth;
    const scale = Math.max(0.01, Math.min(rawScale, maxScaleByHeight, maxScaleByWidth));
    const mirror = this.direction === "left" ? -1 : 1;

    this.screenScale = scale;
    this.model.scale.set(scale * mirror, scale);
    this.model.x = width / 2 + Number(this.config.spine.offsetX || 0);
    this.model.y = usableHeight * 0.96 + Number(this.config.spine.offsetY || 0);
    this.updateAnchor(width, height);
  }

  updateAnchor(stageWidth, stageHeight) {
    if (!this.spine || !(this.fitBounds || this.stableBounds)) return;
    const bounds = this.fitBounds || this.stableBounds;
    const side = this.direction === "left" ? 1 : -1;
    const defaultScale = Math.max(0.01, Number(this.config.spine?.presentationDefaults?.scale || 0.86));
    const presentationRatio = Number(this.config.spine?.scale || 1) / defaultScale;
    const smallModel = presentationRatio < 0.82;
    const tinyModel = presentationRatio < 0.68;
    const horizontalFactor = tinyModel ? 1.05 : smallModel ? 0.78 : 0.38;
    const verticalFactor = tinyModel ? 0.78 : smallModel ? 0.84 : 0.8;
    const anchorScale = Math.max(0.62, Math.min(1.08, 0.56 + presentationRatio * 0.38));
    const x = this.model.x + side * bounds.width * this.screenScale * horizontalFactor;
    const modelTop = this.model.y - bounds.height * this.screenScale;
    const modelWidth = bounds.width * this.screenScale;
    const modelHeight = bounds.height * this.screenScale;
    const modelShoulder = this.model.y - bounds.height * this.screenScale * verticalFactor;
    const minY = Math.max(10, modelTop + 8 * anchorScale);
    const maxY = Math.max(minY, stageHeight - 86 * anchorScale);
    const target = {
      x: Math.max(12, Math.min(stageWidth - 48, x)),
      y: Math.max(minY, Math.min(maxY, modelShoulder)),
      scale: anchorScale,
      side: this.direction === "left" ? "right" : "left",
      avoid: {
        left: this.model.x - modelWidth * 0.34,
        right: this.model.x + modelWidth * 0.34,
        top: this.model.y - modelHeight * 0.72,
        bottom: this.model.y - modelHeight * 0.06
      }
    };

    const canSmooth = this.anchorTarget
      && this.anchorTarget.side === target.side
      && Math.abs(this.anchorTarget.scale - target.scale) < 0.015;
    if (canSmooth) {
      target.y = Math.max(target.y, this.anchorTarget.y - 22);
    }
    const mix = canSmooth ? 0.42 : 1;
    this.anchor = {
      x: this.anchor.x + (target.x - this.anchor.x) * mix,
      y: this.anchor.y + (target.y - this.anchor.y) * mix,
      scale: target.scale,
      side: target.side,
      avoid: target.avoid
    };
    this.anchorTarget = target;
    this.onAnchorChange?.(this.anchor);
  }

  getAnchor() {
    return { ...this.anchor };
  }

  getRuntimeBounds() {
    if (!this.spine?.skeleton?.getBounds) return this.stableBounds;
    try {
      this.runtimeBoundsScratch.length = 0;
      this.spine.skeleton.getBounds(
        this.runtimeBoundsOffset,
        this.runtimeBoundsSize,
        this.runtimeBoundsScratch
      );
      const bounds = {
        x: Number(this.runtimeBoundsOffset.x),
        y: Number(this.runtimeBoundsOffset.y),
        width: Number(this.runtimeBoundsSize.x),
        height: Number(this.runtimeBoundsSize.y)
      };
      return Number.isFinite(bounds.width) && Number.isFinite(bounds.height)
        && bounds.width > 1 && bounds.height > 1
        ? bounds
        : this.stableBounds;
    } catch {
      return this.stableBounds;
    }
  }

  getInteractiveBounds() {
    if (!this.model || !this.spine) return null;
    const sourceBounds = this.getRuntimeBounds();
    if (!sourceBounds) return null;
    const transformed = transformLocalBounds(sourceBounds, {
      x: this.model.x,
      y: this.model.y,
      childX: this.spine.x,
      childY: this.spine.y,
      scaleX: this.model.scale.x,
      scaleY: this.model.scale.y
    });
    if (!transformed) return null;
    return calculateInteractiveBounds({
      width: transformed.width,
      height: transformed.height,
      left: transformed.left,
      top: transformed.top,
      userScale: this.userScale,
      hitboxPadding: this.hitboxPadding
    });
  }

  getInteractiveRegions() {
    if (!this.spine) return [];
    const slotBounds = [];
    for (const slotContainer of this.spine.children || []) {
      if (!slotContainer?.visible || !slotContainer?.renderable || Number(slotContainer.worldAlpha ?? 1) <= 0.01) continue;
      try {
        const bounds = slotContainer.getBounds?.(true);
        if (bounds && Number(bounds.width) > 1 && Number(bounds.height) > 1) {
          slotBounds.push({
            left: Number(bounds.left ?? bounds.x),
            right: Number(bounds.right ?? (bounds.x + bounds.width)),
            top: Number(bounds.top ?? bounds.y),
            bottom: Number(bounds.bottom ?? (bounds.y + bounds.height))
          });
        }
      } catch {}
    }
    const padding = Math.max(2, Math.min(8, Number(this.hitboxPadding || 0) * 0.5));
    const regions = compactPointerRegions(slotBounds, { maxRegions: 16, padding });
    return regions.length ? regions : normalizePointerRegions(this.getInteractiveBounds());
  }

  getPointerRecoveryBounds() {
    const bounds = unionPointerRegions(this.getInteractiveRegions()) || this.getInteractiveBounds();
    if (!bounds) return null;
    const recoveryPadding = Math.max(10, Math.min(24, 12 + this.hitboxPadding * 0.5));
    return expandBounds(bounds, recoveryPadding);
  }

  getRendererHealth() {
    const view = this.app?.view;
    const gl = this.app?.renderer?.gl;
    const contextLost = typeof gl?.isContextLost === "function" ? gl.isContextLost() : false;
    const entry = this.spine?.state?.getCurrent?.(0) || null;
    const trackStale = Boolean(entry?.loop)
      && Number(entry?.timeScale ?? 1) > 0
      && this.lastTrackProgressAt > 0
      && Date.now() >= this.resumeGraceUntil
      && Date.now() - this.lastTrackProgressAt > TRACK_STALE_MS;
    return {
      status: contextLost ? "context-lost" : "ok",
      canvasWidth: Number(view?.width || 0),
      canvasHeight: Number(view?.height || 0),
      clientWidth: Number(view?.clientWidth || 0),
      clientHeight: Number(view?.clientHeight || 0),
      lastFrameAt: this.lastFrameAt,
      tickerStarted: Boolean(this.app?.ticker?.started),
      frameCounter: this.frameCounter,
      animationName: String(entry?.animation?.name || ""),
      trackTime: Number(entry?.trackTime ?? -1),
      animationEnd: Number(entry?.animationEnd ?? entry?.animation?.duration ?? 0),
      loop: Boolean(entry?.loop),
      timeScale: Number(entry?.timeScale ?? 1),
      lastTrackProgressAt: this.lastTrackProgressAt,
      trackStale,
      animationRecoveryCount: this.animationRecoveryHistory.length,
      animationRecoveryStep: this.animationRecoveryStep,
      animationRecoveryExhausted: this.animationRecoveryExhausted,
      lastRecoveryReason: this.lastRecoveryReason || "",
      contextLost,
      hasModel: Boolean(this.spine),
      screenScale: this.screenScale,
      userScale: this.userScale,
      interactiveBounds: this.getInteractiveBounds(),
      recoveryBounds: this.getPointerRecoveryBounds()
    };
  }

  destroy() {
    this.replacementRevision.invalidate();
    window.clearTimeout(this.returnTimer);
    window.clearTimeout(this.resizeTimer);
    window.clearTimeout(this.gpuRecoveryTimer);
    if (this.handleResize) window.removeEventListener("resize", this.handleResize);
    if (this.handleVisibilityChange) document.removeEventListener("visibilitychange", this.handleVisibilityChange);
    if (this.handleWheel) this.stageElement.removeEventListener("wheel", this.handleWheel);
    if (this.app?.view && this.handleContextLost) {
      this.app.view.removeEventListener("webglcontextlost", this.handleContextLost, false);
    }
    if (this.app?.view && this.handleContextRestored) {
      this.app.view.removeEventListener("webglcontextrestored", this.handleContextRestored, false);
    }
    if (this.app && this.frameTicker) this.app.ticker.remove(this.frameTicker);
    if (this.app) {
      const loseContext = this.app.renderer?.gl?.getExtension?.("WEBGL_lose_context");
      this.app.destroy(true, { children: true, texture: false, baseTexture: false });
      this.assetHandle?.release();
      this.assetHandle = null;
      loseContext?.loseContext?.();
    }
    this.app = null;
    this.assetHandle = null;
    this.assetIdentity = "";
    this.spine = null;
    this.model = null;
  }
}
