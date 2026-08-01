use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum RunState {
    Created,
    Planning,
    AwaitingUser,
    Running,
    Pausing,
    Paused,
    WaitingApproval,
    Compacting,
    Verifying,
    Synthesizing,
    Completed,
    PartiallyCompleted,
    Failed,
    Cancelled,
}

impl RunState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::PartiallyCompleted | Self::Failed | Self::Cancelled
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        use RunState as S;
        matches!(
            (self, next),
            (S::Created, S::Planning | S::Cancelled)
                | (
                    S::Planning,
                    S::AwaitingUser | S::Running | S::Failed | S::Cancelled
                )
                | (
                    S::AwaitingUser,
                    S::Planning | S::Running | S::Cancelled | S::Failed
                )
                | (
                    S::Running,
                    S::Pausing
                        | S::WaitingApproval
                        | S::Compacting
                        | S::Verifying
                        | S::Synthesizing
                        | S::Failed
                        | S::Cancelled
                )
                | (S::Pausing, S::Paused | S::Failed | S::Cancelled)
                | (S::Paused, S::Running | S::Cancelled | S::Failed)
                | (
                    S::WaitingApproval,
                    S::Running | S::Planning | S::Cancelled | S::Failed
                )
                | (S::Compacting, S::Running | S::Failed | S::Cancelled)
                | (
                    S::Verifying,
                    S::Running | S::Synthesizing | S::PartiallyCompleted | S::Failed | S::Cancelled
                )
                | (
                    S::Synthesizing,
                    S::Completed | S::PartiallyCompleted | S::Failed | S::Cancelled
                )
        )
    }

    pub fn validate_transition(self, next: Self) -> Result<(), TransitionError> {
        if self.can_transition_to(next) {
            Ok(())
        } else {
            Err(TransitionError::Run {
                from: self,
                to: next,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum WorkerState {
    Draft,
    Ready,
    Queued,
    RunningModel,
    WaitingToolApproval,
    RunningTool,
    WaitingDependency,
    WaitingHandoff,
    Paused,
    Compacting,
    Verifying,
    Completed,
    Failed,
    Cancelled,
}

impl WorkerState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        use WorkerState as S;
        matches!(
            (self, next),
            (S::Draft, S::Ready | S::Cancelled)
                | (S::Ready, S::Queued | S::Cancelled)
                | (
                    S::Queued,
                    S::RunningModel | S::WaitingDependency | S::Paused | S::Cancelled
                )
                | (
                    S::RunningModel,
                    S::WaitingToolApproval
                        | S::RunningTool
                        | S::WaitingDependency
                        | S::WaitingHandoff
                        | S::Compacting
                        | S::Verifying
                        | S::Completed
                        | S::Failed
                        | S::Cancelled
                        | S::Paused
                )
                | (
                    S::WaitingToolApproval,
                    S::RunningTool | S::RunningModel | S::Failed | S::Cancelled
                )
                | (
                    S::RunningTool,
                    S::RunningModel | S::Verifying | S::Failed | S::Cancelled | S::Paused
                )
                | (
                    S::WaitingDependency,
                    S::Queued | S::RunningModel | S::Failed | S::Cancelled | S::Paused
                )
                | (
                    S::WaitingHandoff,
                    S::RunningModel | S::Failed | S::Cancelled | S::Paused
                )
                | (
                    S::Paused,
                    S::Queued | S::RunningModel | S::RunningTool | S::Cancelled | S::Failed
                )
                | (S::Compacting, S::RunningModel | S::Failed | S::Cancelled)
                | (
                    S::Verifying,
                    S::RunningModel | S::Completed | S::Failed | S::Cancelled
                )
        )
    }

    pub fn validate_transition(self, next: Self) -> Result<(), TransitionError> {
        if self.can_transition_to(next) {
            Ok(())
        } else {
            Err(TransitionError::Worker {
                from: self,
                to: next,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TransitionError {
    #[error("invalid run transition: {from:?} -> {to:?}")]
    Run { from: RunState, to: RunState },
    #[error("invalid worker transition: {from:?} -> {to:?}")]
    Worker { from: WorkerState, to: WorkerState },
}
