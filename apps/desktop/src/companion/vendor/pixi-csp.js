import { ShaderSystem } from "@pixi/core";
import { install as installUnsafeEval } from "@pixi/unsafe-eval";

// Pixi 6 generates uniform synchronizers with `new Function` by default. The
// static synchronizer keeps the WebView CSP strict while supporting both the
// pixi.js facade and direct @pixi/core imports.
export function installPixiCspCompatibility() {
  installUnsafeEval({ ShaderSystem });
}

installPixiCspCompatibility();
