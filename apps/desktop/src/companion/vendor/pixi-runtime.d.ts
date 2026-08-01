export class Application {
  constructor(options: Record<string, unknown>);
  view: HTMLCanvasElement;
  renderer: { width: number; height: number; resolution: number };
  stage: { addChild(child: unknown): void };
  destroy(
    removeView?: boolean,
    options?: { children?: boolean; texture?: boolean; baseTexture?: boolean },
  ): void;
}

export class Ticker {}
