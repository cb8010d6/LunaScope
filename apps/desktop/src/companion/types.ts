export type CompanionPhase =
  | "idle"
  | "working"
  | "reviewing"
  | "running"
  | "success"
  | "failed"
  | "waiting";

export type CompanionSettings = {
  enabled: boolean;
  modelPath: string | null;
  modelName: string | null;
  modelKind: "spine38" | "live2d";
  live2dCorePath: string | null;
  scale: number;
  bubbleVisible: boolean;
};

export type CompanionActivity = {
  phase: CompanionPhase;
  title: string;
  detail: string;
};

export type ExecutionActivity = {
  running: boolean;
  workerId: string | null;
  role: string;
  state: string;
  detail: string;
};
