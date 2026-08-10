import * as PIXI from "pixi.js";
import { installPixiCspCompatibility } from "./pixi-csp.js";

installPixiCspCompatibility();

export const Application = PIXI.Application;
export const Ticker = PIXI.Ticker;
