import test from "node:test";
import assert from "node:assert/strict";

import { ShaderSystem } from "pixi.js";
import { ShaderSystem as CoreShaderSystem } from "@pixi/core";
import "./pixi-runtime.js";

test("Pixi runtime installs the CSP-safe shader synchronizer", () => {
  assert.equal(ShaderSystem, CoreShaderSystem);
  assert.equal(ShaderSystem.prototype.systemCheck, CoreShaderSystem.prototype.systemCheck);
  assert.doesNotThrow(() => new ShaderSystem({}));
  assert.doesNotThrow(() => new ShaderSystem({}).systemCheck());
});
