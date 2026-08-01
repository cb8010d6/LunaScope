export class SpinePlayer {
  constructor(stage: HTMLElement, config: Record<string, unknown>);
  init(): Promise<void>;
  applyState(state: { state: string; source?: string }, force?: boolean): void;
  setPresentationScale(scale: number, notify?: boolean): void;
  destroy(): void;
}
