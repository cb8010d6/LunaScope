export class Live2DModel {
  static from(source: string, options?: Record<string, unknown>): Promise<Live2DModel>;
  static registerTicker(ticker: unknown): void;
  anchor: { set(x: number, y?: number): void };
  width: number;
  height: number;
  scale: { set(value: number): void };
  position: { set(x: number, y: number): void };
  internalModel: {
    motionManager?: {
      definitions?: Record<string, unknown[]>;
      once(event: "motionFinish", listener: () => void): void;
    };
  };
  motion(group: string, index?: number, priority?: number): Promise<boolean>;
  hitTest(x: number, y: number): string[];
  containsPoint(point: { x: number; y: number }): boolean;
  getBounds(skipUpdate?: boolean): {
    x: number;
    y: number;
    width: number;
    height: number;
  };
  destroy(options?: unknown): void;
}

export const config: {
  sound: boolean;
  motionSync: boolean;
};
