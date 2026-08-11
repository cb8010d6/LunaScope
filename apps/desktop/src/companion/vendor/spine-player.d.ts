export class SpinePlayer {
  constructor(stage: HTMLElement, config: Record<string, unknown>);
  init(): Promise<void>;
  applyState(state: { state: string; source?: string }, force?: boolean): void;
  playOneShot(
    state: { state: string; source?: string },
    onComplete?: () => void,
  ): unknown | null;
  getInteractiveBounds(): {
    left: number;
    right: number;
    top: number;
    bottom: number;
    width: number;
    height: number;
  } | null;
  setPresentationScale(scale: number, notify?: boolean): void;
  captureFrame(width?: number, height?: number): string | null;
  destroy(): void;
}
