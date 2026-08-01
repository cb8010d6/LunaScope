import type {
  EventData,
  EventEnvelope,
  EventSource,
  RuntimeDelta,
  RuntimeSnapshot,
} from "./types.generated";

const source: EventSource = { kind: "system" };

const payload: EventData = {
  kind: "run_created",
  data: {
    title: "Contract smoke test",
    initial_prompt: "Verify generated types",
  },
};

export const envelope: EventEnvelope = {
  eventId: "evt-1",
  sequence: 1,
  schemaVersion: 1,
  timestamp: "2026-07-27T00:00:00Z",
  projectId: "project-1",
  threadId: "thread-1",
  runId: "run-1",
  orchestrationId: null,
  workerId: null,
  correlationId: "correlation-1",
  causationId: null,
  eventType: "run_created",
  source,
  payload,
  risk: "none",
  redactionState: "not_required",
};

export const snapshot: RuntimeSnapshot = {
  schemaVersion: 1,
  sequence: 1,
  runId: "run-1",
  runState: "created",
  planMarkdown: "",
  orchestrationPlan: null,
  workers: {},
  agentPlans: {},
  reasoningSummaries: {},
  pendingApprovals: [],
  artifactIds: [],
  verification: "unverified",
};

export const delta: RuntimeDelta = {
  fromSequence: 0,
  toSequence: 1,
  events: [envelope],
};
