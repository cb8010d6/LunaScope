use std::sync::Arc;

use lunascope_core::{
    ApprovalRequest, ApprovalResolution, CheckpointId, CheckpointRecord, CorrelationId, EventData,
    EventEnvelope, EventId, EventSource, ModelRoutingDecision, ModelUsageRecord,
    PermissionDecision, ProjectId, RedactionState, RiskLevel, RunId, RunState, RuntimeSnapshot,
    ThreadId, ToolCall, ToolResult, VerificationRecord,
};
use lunascope_storage::{SqliteEventStore, StorageError};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct RunIdentity {
    pub project_id: ProjectId,
    pub thread_id: ThreadId,
    pub run_id: RunId,
    pub correlation_id: CorrelationId,
}

pub struct DurableApprovalRuntime {
    store: Arc<SqliteEventStore>,
    identity: RunIdentity,
}

impl DurableApprovalRuntime {
    pub fn new(store: Arc<SqliteEventStore>, identity: RunIdentity) -> Self {
        Self { store, identity }
    }

    pub fn recover(&self) -> Result<RuntimeSnapshot, ApprovalRuntimeError> {
        self.store
            .recover(&self.identity.run_id)?
            .ok_or_else(|| ApprovalRuntimeError::RunNotFound(self.identity.run_id.clone()))
    }

    pub fn request(
        &self,
        call: ToolCall,
        mut request: ApprovalRequest,
        risk: RiskLevel,
    ) -> Result<RuntimeSnapshot, ApprovalRuntimeError> {
        let snapshot = self.recover()?;
        if snapshot.run_state != RunState::Running {
            return Err(ApprovalRuntimeError::UnexpectedRunState {
                expected: RunState::Running,
                actual: snapshot.run_state,
            });
        }
        if request
            .call_id
            .as_deref()
            .is_some_and(|id| id != call.call_id)
        {
            return Err(ApprovalRuntimeError::CallIdMismatch);
        }
        request.call_id = Some(call.call_id.clone());

        self.store.append_batch_next(vec![
            self.event(
                EventSource::Orchestrator,
                EventData::ToolCallRequested {
                    worker_id: None,
                    call,
                },
                risk,
            ),
            self.event(
                EventSource::Orchestrator,
                EventData::ApprovalRequested {
                    worker_id: None,
                    request,
                },
                risk,
            ),
            self.event(
                EventSource::System,
                EventData::RunStateChanged {
                    from: RunState::Running,
                    to: RunState::WaitingApproval,
                    reason: "tool call requires explicit approval".into(),
                },
                risk,
            ),
        ])?;
        self.store.create_snapshot(&self.identity.run_id)?;
        self.recover()
    }

    pub fn pending_tool_call(&self, approval_id: &str) -> Result<ToolCall, ApprovalRuntimeError> {
        let snapshot = self.recover()?;
        let approval = snapshot
            .pending_approvals
            .iter()
            .find(|request| request.approval_id == approval_id)
            .ok_or_else(|| ApprovalRuntimeError::ApprovalNotFound(approval_id.to_owned()))?;
        let call_id = approval
            .call_id
            .as_deref()
            .ok_or_else(|| ApprovalRuntimeError::ApprovalMissingCall(approval_id.to_owned()))?;
        self.store
            .events_after(&self.identity.run_id, 0)?
            .into_iter()
            .rev()
            .find_map(|event| match event.payload {
                EventData::ToolCallRequested { call, .. } if call.call_id == call_id => Some(call),
                _ => None,
            })
            .ok_or_else(|| ApprovalRuntimeError::ToolCallNotFound(call_id.to_owned()))
    }

    pub fn resolve(
        &self,
        resolution: ApprovalResolution,
    ) -> Result<Option<ToolCall>, ApprovalRuntimeError> {
        let snapshot = self.recover()?;
        if snapshot.run_state != RunState::WaitingApproval {
            return Err(ApprovalRuntimeError::UnexpectedRunState {
                expected: RunState::WaitingApproval,
                actual: snapshot.run_state,
            });
        }
        let call = self.pending_tool_call(&resolution.approval_id)?;
        let rejected = resolution.decision == PermissionDecision::Reject;
        let mut events = vec![self.event(
            EventSource::User,
            EventData::ApprovalResolved {
                resolution: resolution.clone(),
            },
            RiskLevel::Low,
        )];
        if rejected {
            events.push(self.event(
                EventSource::System,
                EventData::ToolCallCompleted {
                    worker_id: None,
                    result: ToolResult {
                        call_id: call.call_id.clone(),
                        success: false,
                        output: serde_json::json!({"rejected": true}),
                        error_code: Some("approval_rejected".into()),
                        duration_ms: 0,
                    },
                },
                RiskLevel::Low,
            ));
        }
        events.push(
            self.event(
                EventSource::System,
                EventData::RunStateChanged {
                    from: RunState::WaitingApproval,
                    to: RunState::Running,
                    reason: if rejected {
                        "approval rejected; return control to planner"
                    } else {
                        "approval granted; tool call may resume"
                    }
                    .into(),
                },
                RiskLevel::Low,
            ),
        );
        self.store.append_batch_next(events)?;
        self.store.create_snapshot(&self.identity.run_id)?;
        Ok((!rejected).then_some(call))
    }

    pub fn checkpoint(
        &self,
        reason: impl Into<String>,
    ) -> Result<RuntimeSnapshot, ApprovalRuntimeError> {
        let snapshot = self.recover()?;
        let checkpoint = CheckpointRecord {
            checkpoint_id: CheckpointId::new(Uuid::new_v4().to_string()),
            event_sequence: snapshot.sequence + 1,
            reason: reason.into(),
            artifact_ids: snapshot.artifact_ids,
        };
        self.store.append_batch_next(vec![self.event(
            EventSource::System,
            EventData::CheckpointCreated { checkpoint },
            RiskLevel::None,
        )])?;
        self.store.create_snapshot(&self.identity.run_id)?;
        self.recover()
    }

    pub fn record_tool_result(
        &self,
        result: ToolResult,
        risk: RiskLevel,
    ) -> Result<RuntimeSnapshot, ApprovalRuntimeError> {
        self.store.append_batch_next(vec![self.event(
            EventSource::Tool("runtime".into()),
            EventData::ToolCallCompleted {
                worker_id: None,
                result,
            },
            risk,
        )])?;
        self.recover()
    }

    pub fn record_model_chunk(
        &self,
        stream_id: impl Into<String>,
        index: u64,
        text: impl Into<String>,
    ) -> Result<RuntimeSnapshot, ApprovalRuntimeError> {
        self.store.append_batch_next(vec![self.event(
            EventSource::Orchestrator,
            EventData::ModelStreamChunk {
                worker_id: None,
                stream_id: stream_id.into(),
                index,
                text: text.into(),
            },
            RiskLevel::None,
        )])?;
        self.recover()
    }

    pub fn record_model_routing(
        &self,
        decision: ModelRoutingDecision,
    ) -> Result<RuntimeSnapshot, ApprovalRuntimeError> {
        self.store.append_batch_next(vec![self.event(
            EventSource::Orchestrator,
            EventData::ModelRoutingDecided { decision },
            RiskLevel::None,
        )])?;
        self.recover()
    }

    pub fn record_model_usage(
        &self,
        usage: ModelUsageRecord,
    ) -> Result<RuntimeSnapshot, ApprovalRuntimeError> {
        self.store.append_batch_next(vec![self.event(
            EventSource::System,
            EventData::ModelUsageRecorded { usage },
            RiskLevel::None,
        )])?;
        self.recover()
    }

    pub fn record_verification(
        &self,
        verification: VerificationRecord,
    ) -> Result<RuntimeSnapshot, ApprovalRuntimeError> {
        self.store.append_batch_next(vec![self.event(
            EventSource::System,
            EventData::VerificationRecorded { verification },
            RiskLevel::None,
        )])?;
        self.recover()
    }

    fn event(&self, source: EventSource, payload: EventData, risk: RiskLevel) -> EventEnvelope {
        let mut event = EventEnvelope::new(
            EventId::new(Uuid::new_v4().to_string()),
            0,
            jiff::Timestamp::now().to_string(),
            self.identity.project_id.clone(),
            self.identity.thread_id.clone(),
            self.identity.run_id.clone(),
            self.identity.correlation_id.clone(),
            source,
            payload,
        );
        event.risk = risk;
        event.redaction_state = RedactionState::NotRequired;
        event
    }
}

#[derive(Debug, Error)]
pub enum ApprovalRuntimeError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("run does not exist: {0}")]
    RunNotFound(RunId),
    #[error("expected run state {expected:?}, got {actual:?}")]
    UnexpectedRunState {
        expected: RunState,
        actual: RunState,
    },
    #[error("approval request call id does not match the tool call")]
    CallIdMismatch,
    #[error("approval does not exist or is no longer pending: {0}")]
    ApprovalNotFound(String),
    #[error("approval has no persisted tool call link: {0}")]
    ApprovalMissingCall(String),
    #[error("persisted tool call was not found: {0}")]
    ToolCallNotFound(String),
}
