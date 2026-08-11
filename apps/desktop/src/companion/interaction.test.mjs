import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  CompanionPointerGesture,
  CompanionPhaseGate,
  placeBubbleNearModel,
  selectLive2DInteractionMotion,
  shouldShowCompanionBubble,
  startLive2DOneShot,
} from "./interaction.ts";

test("a short primary-button press is an interaction, not a drag", () => {
  const gesture = new CompanionPointerGesture(6);

  assert.equal(gesture.begin({ pointerId: 4, button: 0, x: 120, y: 180 }), true);
  assert.equal(gesture.move({ pointerId: 4, x: 123, y: 182 }), "none");
  assert.equal(gesture.end({ pointerId: 4, x: 123, y: 182 }), "interact");
});

test("crossing the movement threshold starts one drag and suppresses interaction", () => {
  const gesture = new CompanionPointerGesture(6);

  gesture.begin({ pointerId: 7, button: 0, x: 50, y: 50 });
  assert.equal(gesture.move({ pointerId: 7, x: 57, y: 50 }), "start-drag");
  assert.equal(gesture.move({ pointerId: 7, x: 80, y: 80 }), "none");
  assert.equal(gesture.end({ pointerId: 7, x: 80, y: 80 }), "none");
});

test("cancelled and non-primary pointers never trigger an interaction", () => {
  const gesture = new CompanionPointerGesture(6);

  assert.equal(gesture.begin({ pointerId: 1, button: 2, x: 0, y: 0 }), false);
  assert.equal(gesture.end({ pointerId: 1, x: 0, y: 0 }), "none");
  gesture.begin({ pointerId: 2, button: 0, x: 0, y: 0 });
  gesture.cancel(2);
  assert.equal(gesture.end({ pointerId: 2, x: 0, y: 0 }), "none");
});

test("the task bubble requires a meaningful non-idle activity", () => {
  assert.equal(
    shouldShowCompanionBubble(true, { phase: "idle", title: "LunaScope", detail: "" }),
    false,
  );
  assert.equal(
    shouldShowCompanionBubble(true, { phase: "working", title: "LunaScope", detail: "   " }),
    false,
  );
  assert.equal(
    shouldShowCompanionBubble(false, { phase: "working", title: "Agent", detail: "Running" }),
    false,
  );
  assert.equal(
    shouldShowCompanionBubble(true, { phase: "working", title: "Agent", detail: "Running" }),
    true,
  );
});

test("Live2D click feedback selects the first supported interaction group", () => {
  assert.equal(selectLive2DInteractionMotion(["Idle", "Special", "TapBody"]), "TapBody");
  assert.equal(selectLive2DInteractionMotion(["idle", "INTERACT"]), "INTERACT");
  assert.equal(selectLive2DInteractionMotion(["Idle", "Wave"]), null);
});

test("Live2D completion is bound only after the new motion has started", async () => {
  let resolveMotion = null;
  const motion = new Promise((resolve) => {
    resolveMotion = resolve;
  });
  const events = [];
  const run = startLive2DOneShot(
    () => motion,
    () => events.push("bind-completion"),
    () => events.push("restore"),
  );

  assert.deepEqual(events, []);
  resolveMotion?.(true);
  await run;
  assert.deepEqual(events, ["bind-completion"]);
});

test("Live2D motion start failure restores the current task phase", async () => {
  const events = [];
  await startLive2DOneShot(
    async () => false,
    () => events.push("bind-completion"),
    () => events.push("restore"),
  );
  assert.deepEqual(events, ["restore"]);
});

test("activity updates cannot interrupt a one-shot interaction", () => {
  const phases = new CompanionPhaseGate();

  assert.equal(phases.requestPhase("working"), true);
  assert.equal(phases.requestPhase("working"), false);
  const interaction = phases.beginInteraction();
  assert.equal(phases.requestPhase("working"), false);
  assert.equal(phases.requestPhase("running"), false);
  assert.equal(phases.finishInteraction(interaction), "running");
  assert.equal(phases.requestPhase("running"), false);
  assert.equal(phases.requestPhase("success"), true);
});

test("only the latest repeated interaction may restore the task phase", () => {
  const phases = new CompanionPhaseGate("reviewing");

  phases.requestPhase("reviewing");
  const first = phases.beginInteraction();
  const second = phases.beginInteraction();
  assert.equal(phases.finishInteraction(first), null);
  assert.equal(phases.finishInteraction(second), "reviewing");
});

test("the task bubble is positioned near the model instead of across the window top", () => {
  assert.deepEqual(
    placeBubbleNearModel(
      { width: 540, height: 680 },
      { width: 220, height: 58 },
      { left: 170, right: 410, top: 350, bottom: 670, width: 240, height: 320 },
    ),
    { left: 180, top: 282, side: "above" },
  );
});

test("the companion bubble hidden attribute cannot be overridden by its grid layout", async () => {
  const styles = await readFile(new URL("./styles.css", import.meta.url), "utf8");
  assert.match(styles, /#companion-bubble\[hidden\]\s*\{[^}]*display:\s*none\s*!important/si);
});
