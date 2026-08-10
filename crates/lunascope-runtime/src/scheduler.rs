use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use lunascope_core::{
    ArtifactRecord, CriterionVerificationStatus, HandoffRecord, OrchestrationPatch,
    OrchestrationPatchApplyMode, OrchestrationPatchOperation, OrchestrationPlan,
    OrchestrationRunResult, OrchestrationValidation, VerificationFinding, VerificationRecord,
    VerificationSeverity, VerificationStatus, WorkerExecutionRecord, WorkerId, WorkerSpec,
    WorkerState, WorkerStateChange,
};
use thiserror::Error;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    PatchArtifact, WorkerWorktree, WorktreeError, WorktreeManager, validate_orchestration,
};

pub type WorkerFuture =
    Pin<Box<dyn Future<Output = Result<WorkerOutput, WorkerFailure>> + Send + 'static>>;

pub trait WorkerExecutor: Send + Sync {
    fn execute(&self, context: WorkerExecutionContext) -> WorkerFuture;
}

#[derive(Clone, Debug)]
pub struct SchedulerControl {
    inner: Arc<Mutex<SchedulerControlState>>,
    resumed: Arc<Notify>,
}

#[derive(Clone, Debug)]
struct WorkerRevision {
    mode: OrchestrationPatchApplyMode,
    spec: WorkerSpec,
}

#[derive(Debug)]
struct SchedulerControlState {
    current_plan: OrchestrationPlan,
    topology_revision: Option<OrchestrationPlan>,
    states: BTreeMap<WorkerId, WorkerState>,
    revisions: BTreeMap<WorkerId, VecDeque<WorkerRevision>>,
    guidance: BTreeMap<WorkerId, VecDeque<String>>,
    paused: bool,
}

impl SchedulerControl {
    pub fn new(plan: &OrchestrationPlan) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SchedulerControlState {
                current_plan: plan.clone(),
                topology_revision: None,
                states: plan
                    .workers
                    .iter()
                    .map(|worker| (worker.worker_id.clone(), WorkerState::Draft))
                    .collect(),
                revisions: BTreeMap::new(),
                guidance: BTreeMap::new(),
                paused: false,
            })),
            resumed: Arc::new(Notify::new()),
        }
    }

    pub fn worker_state(&self, worker_id: &WorkerId) -> Option<WorkerState> {
        self.inner.lock().ok()?.states.get(worker_id).copied()
    }

    pub fn current_plan(&self) -> Option<OrchestrationPlan> {
        Some(self.inner.lock().ok()?.current_plan.clone())
    }

    pub fn pause(&self) -> Result<bool, SchedulerControlError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| SchedulerControlError::Poisoned)?;
        let changed = !inner.paused;
        inner.paused = true;
        Ok(changed)
    }

    pub fn resume(&self) -> Result<bool, SchedulerControlError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| SchedulerControlError::Poisoned)?;
        let changed = inner.paused;
        inner.paused = false;
        drop(inner);
        if changed {
            self.resumed.notify_waiters();
        }
        Ok(changed)
    }

    pub fn is_paused(&self) -> bool {
        self.inner.lock().map(|inner| inner.paused).unwrap_or(false)
    }

    pub async fn wait_if_paused(&self, cancellation: &CancellationToken) -> bool {
        loop {
            if cancellation.is_cancelled() {
                return false;
            }
            let notified = self.resumed.notified();
            if !self.is_paused() {
                return true;
            }
            tokio::select! {
                _ = cancellation.cancelled() => return false,
                _ = notified => {}
            }
        }
    }

    pub fn submit_guidance(
        &self,
        worker_ids: impl IntoIterator<Item = WorkerId>,
        guidance: String,
    ) -> Result<(), SchedulerControlError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| SchedulerControlError::Poisoned)?;
        for worker_id in worker_ids {
            if !inner.states.contains_key(&worker_id) {
                return Err(SchedulerControlError::WorkerMissing(worker_id));
            }
            inner
                .guidance
                .entry(worker_id)
                .or_default()
                .push_back(guidance.clone());
        }
        Ok(())
    }

    pub fn take_guidance(&self, worker_id: &WorkerId) -> Vec<String> {
        let Ok(mut inner) = self.inner.lock() else {
            return Vec::new();
        };
        inner
            .guidance
            .remove(worker_id)
            .map(VecDeque::into_iter)
            .map(Iterator::collect)
            .unwrap_or_default()
    }

    pub fn submit_revision(
        &self,
        patch: &OrchestrationPatch,
        plan: &OrchestrationPlan,
    ) -> Result<(), SchedulerControlError> {
        if !matches!(
            patch.apply_mode,
            OrchestrationPatchApplyMode::ApplyNow
                | OrchestrationPatchApplyMode::ApplyAfterCurrentStep
                | OrchestrationPatchApplyMode::ApplyOnRetry
        ) {
            return Err(SchedulerControlError::UnsupportedMode);
        }
        let topology_change = patch.operations.iter().any(|operation| match operation {
            OrchestrationPatchOperation::AddWorker { .. }
            | OrchestrationPatchOperation::RemoveWorker { .. } => true,
            OrchestrationPatchOperation::UpdateWorker { patch } => {
                patch.dependencies.is_some() || patch.parent_worker_id.is_some()
            }
        });
        if topology_change {
            if !matches!(
                patch.apply_mode,
                OrchestrationPatchApplyMode::ApplyNow
                    | OrchestrationPatchApplyMode::ApplyAfterCurrentStep
            ) {
                return Err(SchedulerControlError::UnsupportedMode);
            }
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| SchedulerControlError::Poisoned)?;
            for operation in &patch.operations {
                match operation {
                    OrchestrationPatchOperation::RemoveWorker { worker_id } => {
                        let state = inner.states.get(worker_id).copied().ok_or_else(|| {
                            SchedulerControlError::WorkerMissing(worker_id.clone())
                        })?;
                        if !matches!(
                            state,
                            WorkerState::Draft
                                | WorkerState::Ready
                                | WorkerState::Queued
                                | WorkerState::WaitingDependency
                        ) {
                            return Err(SchedulerControlError::InvalidWorkerState {
                                worker_id: worker_id.clone(),
                                mode: patch.apply_mode,
                                state,
                            });
                        }
                    }
                    OrchestrationPatchOperation::UpdateWorker { patch: update } => {
                        let state =
                            inner
                                .states
                                .get(&update.worker_id)
                                .copied()
                                .ok_or_else(|| {
                                    SchedulerControlError::WorkerMissing(update.worker_id.clone())
                                })?;
                        if (update.dependencies.is_some() || update.parent_worker_id.is_some())
                            && !matches!(
                                state,
                                WorkerState::Draft
                                    | WorkerState::Ready
                                    | WorkerState::Queued
                                    | WorkerState::WaitingDependency
                            )
                        {
                            return Err(SchedulerControlError::InvalidWorkerState {
                                worker_id: update.worker_id.clone(),
                                mode: patch.apply_mode,
                                state,
                            });
                        }
                    }
                    OrchestrationPatchOperation::AddWorker { spec } => {
                        if inner.states.contains_key(&spec.worker_id) {
                            return Err(SchedulerControlError::WorkerMissing(
                                spec.worker_id.clone(),
                            ));
                        }
                    }
                }
            }
            for operation in &patch.operations {
                match operation {
                    OrchestrationPatchOperation::RemoveWorker { worker_id } => {
                        inner
                            .states
                            .insert(worker_id.clone(), WorkerState::Cancelled);
                        inner.guidance.remove(worker_id);
                        inner.revisions.remove(worker_id);
                    }
                    OrchestrationPatchOperation::AddWorker { spec } => {
                        inner
                            .states
                            .insert(spec.worker_id.clone(), WorkerState::Draft);
                    }
                    OrchestrationPatchOperation::UpdateWorker { .. } => {}
                }
            }
            inner.current_plan = plan.clone();
            inner.topology_revision = Some(plan.clone());
            return Ok(());
        }
        let mut affected = Vec::new();
        for operation in &patch.operations {
            let OrchestrationPatchOperation::UpdateWorker { patch: update } = operation else {
                return Err(SchedulerControlError::TopologyRequiresClone);
            };
            if update.dependencies.is_some() || update.parent_worker_id.is_some() {
                return Err(SchedulerControlError::TopologyRequiresClone);
            }
            let spec = plan
                .workers
                .iter()
                .find(|worker| worker.worker_id == update.worker_id)
                .cloned()
                .ok_or_else(|| SchedulerControlError::WorkerMissing(update.worker_id.clone()))?;
            affected.push(spec);
        }
        if affected.is_empty() {
            return Err(SchedulerControlError::NoWorkerUpdates);
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| SchedulerControlError::Poisoned)?;
        for spec in &affected {
            let state = inner
                .states
                .get(&spec.worker_id)
                .copied()
                .ok_or_else(|| SchedulerControlError::WorkerMissing(spec.worker_id.clone()))?;
            let allowed = match patch.apply_mode {
                OrchestrationPatchApplyMode::ApplyNow => {
                    matches!(state, WorkerState::Queued | WorkerState::WaitingDependency)
                }
                OrchestrationPatchApplyMode::ApplyAfterCurrentStep => {
                    matches!(state, WorkerState::Queued | WorkerState::WaitingDependency)
                }
                OrchestrationPatchApplyMode::ApplyOnRetry => {
                    matches!(state, WorkerState::RunningModel | WorkerState::Paused)
                }
                _ => false,
            };
            if !allowed {
                return Err(SchedulerControlError::InvalidWorkerState {
                    worker_id: spec.worker_id.clone(),
                    mode: patch.apply_mode,
                    state,
                });
            }
        }
        for spec in affected {
            inner
                .revisions
                .entry(spec.worker_id.clone())
                .or_default()
                .push_back(WorkerRevision {
                    mode: patch.apply_mode,
                    spec,
                });
        }
        inner.current_plan = plan.clone();
        drop(inner);
        Ok(())
    }

    fn mark_state(&self, worker_id: &WorkerId, state: WorkerState) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.states.insert(worker_id.clone(), state);
        }
    }

    fn take_dispatch_revision(&self, worker_id: &WorkerId) -> Option<WorkerRevision> {
        self.take_revision(worker_id, |mode| {
            matches!(
                mode,
                OrchestrationPatchApplyMode::ApplyNow
                    | OrchestrationPatchApplyMode::ApplyAfterCurrentStep
            )
        })
    }

    fn take_topology_revision(&self) -> Option<OrchestrationPlan> {
        self.inner.lock().ok()?.topology_revision.take()
    }

    fn take_retry_revision(&self, worker_id: &WorkerId) -> Option<WorkerRevision> {
        self.take_revision(worker_id, |mode| {
            mode == OrchestrationPatchApplyMode::ApplyOnRetry
        })
    }

    fn take_revision(
        &self,
        worker_id: &WorkerId,
        accepts: impl Fn(OrchestrationPatchApplyMode) -> bool,
    ) -> Option<WorkerRevision> {
        let mut inner = self.inner.lock().ok()?;
        let revisions = inner.revisions.get_mut(worker_id)?;
        let position = revisions.iter().position(|item| accepts(item.mode))?;
        revisions.remove(position)
    }
}

#[derive(Debug, Error)]
pub enum SchedulerControlError {
    #[error("active graph topology changes require Clone revision")]
    TopologyRequiresClone,
    #[error("this patch mode is not handled by the live scheduler")]
    UnsupportedMode,
    #[error("active patch contains no Worker update")]
    NoWorkerUpdates,
    #[error("worker is missing from the active scheduler: {0}")]
    WorkerMissing(WorkerId),
    #[error("{mode:?} cannot update {worker_id} while it is {state:?}")]
    InvalidWorkerState {
        worker_id: WorkerId,
        mode: OrchestrationPatchApplyMode,
        state: WorkerState,
    },
    #[error("active scheduler control is unavailable")]
    Poisoned,
}

#[derive(Clone, Debug)]
pub struct WorkerExecutionContext {
    pub spec: WorkerSpec,
    pub attempt: u32,
    /// The canonical task-wide AC contract. Workers receive the same stable IDs,
    /// and the final Verifier must return one evidence-bearing result per item.
    pub acceptance_contract: Vec<String>,
    /// The exact terminal observation from the previous attempt. Retried Workers must
    /// resume from this evidence instead of rediscovering the failure from scratch.
    pub previous_failure: Option<WorkerFailure>,
    /// A downstream writable repair Worker directly depends on this Worker, so a
    /// bounded implementation phase may hand off an observed defect without losing
    /// its already-created workspace changes.
    pub can_handoff_incomplete: bool,
    pub worktree_path: PathBuf,
    pub write_through_workspace_path: Option<PathBuf>,
    pub workspace_snapshot: Option<String>,
    pub input_artifacts: Vec<StoredWorkerArtifact>,
    pub cancellation: CancellationToken,
    pub control: Arc<SchedulerControl>,
}

#[derive(Clone, Debug)]
pub struct StoredWorkerArtifact {
    pub record: ArtifactRecord,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ProducedWorkerArtifact {
    pub name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct WorkerOutput {
    pub summary: String,
    pub artifacts: Vec<ProducedWorkerArtifact>,
    pub verification: Option<VerificationRecord>,
}

#[derive(Clone, Debug, Error)]
#[error("{code}: {message}")]
pub struct WorkerFailure {
    pub code: String,
    pub message: String,
}

impl WorkerFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone)]
pub struct AgentScheduler {
    worktrees: WorktreeManager,
    integrate_workspace_changes: bool,
}

impl AgentScheduler {
    pub fn new(worktrees: WorktreeManager) -> Self {
        Self {
            worktrees,
            integrate_workspace_changes: false,
        }
    }

    pub fn new_integrating(worktrees: WorktreeManager) -> Self {
        Self {
            worktrees,
            integrate_workspace_changes: true,
        }
    }

    pub async fn run(
        &self,
        plan: &OrchestrationPlan,
        executor: Arc<dyn WorkerExecutor>,
        cancellation: CancellationToken,
    ) -> Result<OrchestrationRunResult, SchedulerError> {
        self.run_controlled(
            plan,
            executor,
            cancellation,
            Arc::new(SchedulerControl::new(plan)),
        )
        .await
    }

    pub async fn run_controlled(
        &self,
        plan: &OrchestrationPlan,
        executor: Arc<dyn WorkerExecutor>,
        cancellation: CancellationToken,
        control: Arc<SchedulerControl>,
    ) -> Result<OrchestrationRunResult, SchedulerError> {
        let mut current_plan = plan.clone();
        let mut validation = validate_orchestration(&current_plan);
        if !validation.valid {
            return Err(SchedulerError::InvalidPlan(validation));
        }
        let mut by_id = current_plan
            .workers
            .iter()
            .map(|worker| (worker.worker_id.clone(), worker.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut records = BTreeMap::new();
        let mut state_changes = Vec::new();
        for worker in &plan.workers {
            let mut state = WorkerState::Draft;
            transition(
                &worker.worker_id,
                &mut state,
                WorkerState::Ready,
                "WorkerSpec validated",
                &mut state_changes,
            )?;
            transition(
                &worker.worker_id,
                &mut state,
                WorkerState::Queued,
                "queued by dependency scheduler",
                &mut state_changes,
            )?;
            if !worker.dependencies.is_empty() {
                transition(
                    &worker.worker_id,
                    &mut state,
                    WorkerState::WaitingDependency,
                    "waiting for dependency artifacts",
                    &mut state_changes,
                )?;
            }
            records.insert(
                worker.worker_id.clone(),
                WorkerExecutionRecord {
                    worker_id: worker.worker_id.clone(),
                    state,
                    attempts: 0,
                    worktree_path: None,
                    artifact_ids: Vec::new(),
                    summary: String::new(),
                    error_code: None,
                },
            );
            control.mark_state(&worker.worker_id, state);
        }

        let mut pending = plan
            .workers
            .iter()
            .map(|worker| worker.worker_id.clone())
            .collect::<BTreeSet<_>>();
        let mut completed = BTreeMap::<WorkerId, CompletedWorker>::new();
        let mut failed = BTreeSet::<WorkerId>::new();
        let mut unavailable = BTreeSet::<WorkerId>::new();
        let mut artifacts = Vec::<StoredWorkerArtifact>::new();
        let mut patches = BTreeMap::<WorkerId, PatchArtifact>::new();
        let mut handoffs = Vec::new();
        let mut verification_by_worker = BTreeMap::<WorkerId, VerificationRecord>::new();
        let mut running = FuturesUnordered::new();

        while !pending.is_empty() || !running.is_empty() {
            if let Some(revised_plan) = control.take_topology_revision() {
                let revised_validation = validate_orchestration(&revised_plan);
                if !revised_validation.valid {
                    return Err(SchedulerError::InvalidPlan(revised_validation));
                }
                let revised_by_id = revised_plan
                    .workers
                    .iter()
                    .map(|worker| (worker.worker_id.clone(), worker.clone()))
                    .collect::<BTreeMap<_, _>>();

                let removed = by_id
                    .keys()
                    .filter(|worker_id| !revised_by_id.contains_key(*worker_id))
                    .cloned()
                    .collect::<Vec<_>>();
                for worker_id in removed {
                    if pending.remove(&worker_id) {
                        let record = records
                            .get_mut(&worker_id)
                            .ok_or(SchedulerError::WorkerMissing)?;
                        transition(
                            &worker_id,
                            &mut record.state,
                            WorkerState::Cancelled,
                            "removed by an accepted orchestration revision",
                            &mut state_changes,
                        )?;
                        record.error_code = Some("replanned".into());
                    }
                }

                for worker in &revised_plan.workers {
                    if !by_id.contains_key(&worker.worker_id) {
                        let mut state = WorkerState::Draft;
                        transition(
                            &worker.worker_id,
                            &mut state,
                            WorkerState::Ready,
                            "WorkerSpec added by orchestration revision",
                            &mut state_changes,
                        )?;
                        transition(
                            &worker.worker_id,
                            &mut state,
                            WorkerState::Queued,
                            "queued by revised dependency scheduler",
                            &mut state_changes,
                        )?;
                        if !worker.dependencies.is_empty() {
                            transition(
                                &worker.worker_id,
                                &mut state,
                                WorkerState::WaitingDependency,
                                "waiting for revised dependency artifacts",
                                &mut state_changes,
                            )?;
                        }
                        records.insert(
                            worker.worker_id.clone(),
                            WorkerExecutionRecord {
                                worker_id: worker.worker_id.clone(),
                                state,
                                attempts: 0,
                                worktree_path: None,
                                artifact_ids: Vec::new(),
                                summary: String::new(),
                                error_code: None,
                            },
                        );
                        pending.insert(worker.worker_id.clone());
                        control.mark_state(&worker.worker_id, state);
                    }
                }

                current_plan = revised_plan;
                validation = revised_validation;
                by_id = revised_by_id;
            }
            if cancellation.is_cancelled() {
                cancel_pending(
                    &pending,
                    &mut records,
                    &mut state_changes,
                    "orchestration cancelled",
                )?;
                break;
            }
            if control.is_paused()
                && running.is_empty()
                && !control.wait_if_paused(&cancellation).await
            {
                cancel_pending(
                    &pending,
                    &mut records,
                    &mut state_changes,
                    "orchestration cancelled while paused",
                )?;
                break;
            }
            let blocked = pending
                .iter()
                .filter(|id| {
                    by_id.get(*id).is_some_and(|worker| {
                        worker
                            .dependencies
                            .iter()
                            .any(|dep| unavailable.contains(dep))
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            let propagated_dependency_failure = !blocked.is_empty();
            for id in blocked {
                let record = records.get_mut(&id).ok_or(SchedulerError::WorkerMissing)?;
                transition(
                    &id,
                    &mut record.state,
                    WorkerState::Cancelled,
                    "a required dependency failed",
                    &mut state_changes,
                )?;
                record.error_code = Some("dependency_failed".into());
                unavailable.insert(id.clone());
                pending.remove(&id);
            }
            if propagated_dependency_failure {
                // Re-evaluate the graph before declaring a stall. A dependency failure can
                // cancel several layers of descendants, and each layer becomes visible only
                // after its parent is added to `unavailable`.
                continue;
            }
            let available_slots = if control.is_paused() {
                0
            } else {
                (current_plan.maximum_parallel_workers as usize).saturating_sub(running.len())
            };
            let ready = pending
                .iter()
                .filter(|id| {
                    by_id.get(*id).is_some_and(|worker| {
                        worker
                            .dependencies
                            .iter()
                            .all(|dependency| completed.contains_key(dependency))
                    })
                })
                .take(available_slots)
                .cloned()
                .collect::<Vec<_>>();
            if ready.is_empty() && running.is_empty() {
                if pending.is_empty() {
                    break;
                }
                return Err(SchedulerError::SchedulerStalled);
            }

            for worker_id in ready {
                pending.remove(&worker_id);
                let mut spec = by_id
                    .get(&worker_id)
                    .cloned()
                    .ok_or(SchedulerError::WorkerMissing)?;
                if let Some(revision) = control.take_dispatch_revision(&worker_id) {
                    spec = revision.spec;
                }
                control.mark_state(&worker_id, WorkerState::RunningModel);
                let dependency_artifacts = dependency_artifacts(&spec, &completed);
                for dependency in &spec.dependencies {
                    let dependency_ids = completed
                        .get(dependency)
                        .map(|worker| {
                            worker
                                .artifacts
                                .iter()
                                .map(|artifact| artifact.record.artifact_id.clone())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    handoffs.push(HandoffRecord {
                        handoff_id: format!("handoff-{}", Uuid::new_v4()),
                        from_worker_id: dependency.clone(),
                        to_worker_id: worker_id.clone(),
                        artifact_ids: dependency_ids,
                        summary: format!(
                            "Dependency {} completed; immutable artifacts supplied to {}",
                            dependency, worker_id
                        ),
                    });
                }
                let dependency_patches = ancestor_patches(
                    &current_plan,
                    &worker_id,
                    &validation.topological_order,
                    &patches,
                );
                let manager = self.worktrees.clone();
                let executor = executor.clone();
                let worker_cancellation = cancellation.clone();
                let worker_control = control.clone();
                let integrate_workspace_changes = self.integrate_workspace_changes;
                let acceptance_contract = current_plan.user_hard_constraints.clone();
                let can_handoff_incomplete = current_plan.workers.iter().any(|candidate| {
                    candidate.role.eq_ignore_ascii_case("reviewer")
                        && candidate
                            .tools
                            .iter()
                            .any(|tool| tool == "filesystem.patch")
                        && candidate.dependencies.contains(&worker_id)
                });
                running.push(async move {
                    let result = run_worker(
                        spec,
                        dependency_artifacts,
                        dependency_patches,
                        WorkerRunContext {
                            manager,
                            executor,
                            cancellation: worker_cancellation,
                            control: worker_control,
                            integrate_workspace_changes,
                            can_handoff_incomplete,
                            acceptance_contract,
                        },
                    )
                    .await;
                    (worker_id, result)
                });
            }

            if let Some((worker_id, outcome)) = running.next().await {
                match outcome {
                    Ok(outcome) => {
                        let record = records
                            .get_mut(&worker_id)
                            .ok_or(SchedulerError::WorkerMissing)?;
                        for change in outcome.state_changes {
                            record.state = change.to;
                            state_changes.push(change);
                        }
                        record.attempts = outcome.attempts;
                        record.worktree_path =
                            Some(outcome.worktree.path.to_string_lossy().into_owned());
                        record.artifact_ids = outcome
                            .artifacts
                            .iter()
                            .map(|artifact| artifact.record.artifact_id.clone())
                            .collect();
                        record.summary = outcome.summary.clone();
                        if let Some(patch) = outcome.patch {
                            patches.insert(worker_id.clone(), patch);
                        }
                        if let Some(verification) = outcome.verification.clone() {
                            verification_by_worker.insert(worker_id.clone(), verification);
                        }
                        artifacts.extend(outcome.artifacts.clone());
                        completed.insert(
                            worker_id.clone(),
                            CompletedWorker {
                                artifacts: outcome.artifacts,
                                summary: outcome.summary,
                            },
                        );
                        control.mark_state(&worker_id, WorkerState::Completed);
                    }
                    Err(failure) => {
                        let user_cancelled = cancellation.is_cancelled()
                            || failure.code.eq_ignore_ascii_case("cancelled");
                        let record = records
                            .get_mut(&worker_id)
                            .ok_or(SchedulerError::WorkerMissing)?;
                        for mut change in failure.state_changes {
                            if user_cancelled && change.to == WorkerState::Failed {
                                change.to = WorkerState::Cancelled;
                                change.reason = "orchestration cancelled by the user".into();
                            }
                            record.state = change.to;
                            state_changes.push(change);
                        }
                        record.attempts = failure.attempts;
                        record.worktree_path = failure.worktree_path;
                        record.summary = if user_cancelled {
                            "orchestration cancelled by the user".into()
                        } else {
                            failure.message
                        };
                        record.error_code = Some(if user_cancelled {
                            "cancelled".into()
                        } else {
                            failure.code
                        });
                        control.mark_state(&worker_id, record.state);
                        if !user_cancelled {
                            failed.insert(worker_id.clone());
                            unavailable.insert(worker_id);
                        }
                    }
                }
            }
        }

        let final_plan = control.current_plan().unwrap_or_else(|| plan.clone());
        let user_cancelled = cancellation.is_cancelled()
            || records
                .values()
                .any(|record| record.error_code.as_deref() == Some("cancelled"));
        let mut verification = if user_cancelled {
            VerificationRecord {
                status: VerificationStatus::UnableToVerify,
                summary: "Verification was not run because the user cancelled the orchestration."
                    .into(),
                evidence: state_changes
                    .iter()
                    .filter(|change| change.to == WorkerState::Completed)
                    .map(|change| format!("{}: {}", change.worker_id, change.reason))
                    .collect(),
                remaining_risks: vec!["the cancelled task may be incomplete".into()],
                criterion_results: Vec::new(),
                findings: Vec::new(),
            }
        } else if failed.is_empty() {
            final_plan
                .workers
                .iter()
                .rev()
                .find(|worker| worker.role.eq_ignore_ascii_case("verifier"))
                .and_then(|worker| verification_by_worker.get(&worker.worker_id))
                .cloned()
                .unwrap_or_else(|| VerificationRecord {
                    status: VerificationStatus::PartiallyVerified,
                    summary: "Workers completed, but no independent Verifier record was produced."
                        .into(),
                    evidence: artifacts
                        .iter()
                        .map(|artifact| artifact.record.artifact_id.to_string())
                        .collect(),
                    remaining_risks: vec!["independent verification was not run".into()],
                    criterion_results: Vec::new(),
                    findings: Vec::new(),
                })
        } else {
            VerificationRecord {
                status: VerificationStatus::FailedVerification,
                summary: format!(
                    "{} worker(s) failed or were dependency-cancelled.",
                    failed.len()
                ),
                evidence: state_changes
                    .iter()
                    .filter(|change| {
                        matches!(change.to, WorkerState::Failed | WorkerState::Cancelled)
                    })
                    .map(|change| format!("{}: {}", change.worker_id, change.reason))
                    .collect(),
                remaining_risks: vec!["failed worker objectives remain unresolved".into()],
                criterion_results: Vec::new(),
                findings: Vec::new(),
            }
        };
        if failed.is_empty() && !user_cancelled {
            enforce_acceptance_coverage(&mut verification, &final_plan.user_hard_constraints);
        }
        let synthesis = validation
            .topological_order
            .iter()
            .filter_map(|id| completed.get(id).map(|worker| (id, &worker.summary)))
            .map(|(id, summary)| format!("{id}: {summary}"))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(OrchestrationRunResult {
            orchestration_id: final_plan.orchestration_id,
            version: final_plan.version,
            workers: records,
            state_changes,
            artifacts: artifacts
                .into_iter()
                .map(|artifact| artifact.record)
                .collect(),
            handoffs,
            synthesis,
            verification,
        })
    }
}

fn enforce_acceptance_coverage(
    verification: &mut VerificationRecord,
    acceptance_contract: &[String],
) {
    if acceptance_contract.is_empty() {
        return;
    }

    let mut seen = BTreeSet::new();
    let mut fatal_findings = Vec::new();
    for result in &verification.criterion_results {
        let normalized = result.criterion_id.trim().to_ascii_uppercase();
        if !seen.insert(normalized.clone()) {
            fatal_findings.push(VerificationFinding {
                severity: VerificationSeverity::Fatal,
                title: format!("Duplicate acceptance result: {normalized}"),
                description: "The independent verdict returned more than one result for the same acceptance criterion, so coverage is ambiguous.".into(),
                affected_paths: Vec::new(),
                repair_hint: format!("Re-run the verifier and return exactly one observed result for {normalized}."),
            });
        }
    }

    for (index, criterion) in acceptance_contract.iter().enumerate() {
        let criterion_id = format!("AC-{}", index + 1);
        let matches = verification
            .criterion_results
            .iter()
            .filter(|result| {
                result
                    .criterion_id
                    .trim()
                    .eq_ignore_ascii_case(&criterion_id)
            })
            .collect::<Vec<_>>();
        let Some(result) = matches.first() else {
            fatal_findings.push(VerificationFinding {
                severity: VerificationSeverity::Fatal,
                title: format!("Missing acceptance evidence: {criterion_id}"),
                description: format!("The Verifier did not return a criterion-level result for `{criterion}`."),
                affected_paths: Vec::new(),
                repair_hint: format!("Inspect the current workspace against {criterion_id}, run the smallest check that can prove `{criterion}`, repair any defect, and return direct evidence."),
            });
            continue;
        };
        match result.status {
            CriterionVerificationStatus::Passed if result.evidence.is_empty() => {
                fatal_findings.push(VerificationFinding {
                    severity: VerificationSeverity::Fatal,
                    title: format!("Unsupported acceptance claim: {criterion_id}"),
                    description: format!("The Verifier marked `{criterion}` as passed without direct evidence."),
                    affected_paths: Vec::new(),
                    repair_hint: format!("Run and cite an observable check for {criterion_id}; a global summary or file-existence claim is insufficient."),
                });
            }
            CriterionVerificationStatus::Failed | CriterionVerificationStatus::Unverified => {
                fatal_findings.push(VerificationFinding {
                    severity: VerificationSeverity::Fatal,
                    title: format!("Acceptance criterion not satisfied: {criterion_id}"),
                    description: format!("`{criterion}` was reported as {:?}: {}", result.status, result.note),
                    affected_paths: Vec::new(),
                    repair_hint: format!("Use the recorded {criterion_id} evidence to create a targeted repair and re-run only that criterion."),
                });
            }
            CriterionVerificationStatus::Passed => {}
        }
    }

    if !fatal_findings.is_empty() {
        verification.status = VerificationStatus::FailedVerification;
        verification.findings.extend(fatal_findings);
        if !verification
            .remaining_risks
            .iter()
            .any(|risk| risk == "task-wide acceptance coverage is incomplete")
        {
            verification
                .remaining_risks
                .push("task-wide acceptance coverage is incomplete".into());
        }
    } else if verification.status == VerificationStatus::PartiallyVerified
        && verification.findings.is_empty()
    {
        // A complete criterion ledger is the authoritative definition of done.
        // Some Providers still add a generic residual-risk sentence while also
        // reporting every criterion as directly evidenced and passed. Resolve
        // that contradiction deterministically instead of trapping a healthy
        // run in a repair loop with no failed scope to repair.
        verification.status = VerificationStatus::Verified;
        verification.remaining_risks.clear();
    }
}

#[derive(Clone, Debug)]
struct CompletedWorker {
    artifacts: Vec<StoredWorkerArtifact>,
    summary: String,
}

struct SuccessfulWorker {
    attempts: u32,
    worktree: WorkerWorktree,
    artifacts: Vec<StoredWorkerArtifact>,
    patch: Option<PatchArtifact>,
    summary: String,
    verification: Option<VerificationRecord>,
    state_changes: Vec<WorkerStateChange>,
}

struct FailedWorker {
    attempts: u32,
    worktree_path: Option<String>,
    code: String,
    message: String,
    state_changes: Vec<WorkerStateChange>,
}

struct WorkerRunContext {
    manager: WorktreeManager,
    executor: Arc<dyn WorkerExecutor>,
    cancellation: CancellationToken,
    control: Arc<SchedulerControl>,
    integrate_workspace_changes: bool,
    can_handoff_incomplete: bool,
    acceptance_contract: Vec<String>,
}

async fn run_worker(
    mut spec: WorkerSpec,
    input_artifacts: Vec<StoredWorkerArtifact>,
    dependency_patches: Vec<PatchArtifact>,
    runtime: WorkerRunContext,
) -> Result<SuccessfulWorker, FailedWorker> {
    let WorkerRunContext {
        manager,
        executor,
        cancellation,
        control,
        integrate_workspace_changes,
        can_handoff_incomplete,
        acceptance_contract,
    } = runtime;
    let initial_state = if spec.dependencies.is_empty() {
        WorkerState::Queued
    } else {
        WorkerState::WaitingDependency
    };
    let mut state = initial_state;
    let mut state_changes = Vec::new();
    let mut last_worktree = None;
    let mut resume_patches = Vec::new();
    let mut previous_failure = None;
    let maximum_attempts = spec.retry_policy.maximum_attempts;
    for attempt in 1..=maximum_attempts {
        if !control.wait_if_paused(&cancellation).await {
            return fail_worker(
                &spec.worker_id,
                &mut state,
                "cancelled",
                "worker cancelled while paused".into(),
                attempt,
                last_worktree,
                state_changes,
            );
        }
        if attempt > 1
            && let Some(revision) = control.take_retry_revision(&spec.worker_id)
        {
            spec = revision.spec;
        }
        if (state == WorkerState::WaitingDependency || state == WorkerState::Paused)
            && let Err(error) = transition(
                &spec.worker_id,
                &mut state,
                WorkerState::Queued,
                if attempt == 1 {
                    "dependency artifacts received"
                } else {
                    "retry queued"
                },
                &mut state_changes,
            )
        {
            return failed_from_scheduler(error, attempt, last_worktree, state_changes);
        }
        if let Err(error) = transition(
            &spec.worker_id,
            &mut state,
            WorkerState::RunningModel,
            format!("attempt {attempt} dispatched; isolated worktree preparation started"),
            &mut state_changes,
        ) {
            return failed_from_scheduler(error, attempt, last_worktree, state_changes);
        }
        control.mark_state(&spec.worker_id, WorkerState::RunningModel);
        let worktree = match manager
            .prepare_with_resume(
                &spec.worker_id,
                attempt,
                &dependency_patches,
                &resume_patches,
                cancellation.clone(),
            )
            .await
        {
            Ok(worktree) => worktree,
            Err(error) => {
                return fail_worker(
                    &spec.worker_id,
                    &mut state,
                    "worktree",
                    error.to_string(),
                    attempt,
                    last_worktree,
                    state_changes,
                );
            }
        };
        last_worktree = Some(worktree.path.to_string_lossy().into_owned());
        let context = WorkerExecutionContext {
            spec: spec.clone(),
            attempt,
            acceptance_contract: acceptance_contract.clone(),
            previous_failure: previous_failure.clone(),
            can_handoff_incomplete,
            worktree_path: worktree.path.clone(),
            write_through_workspace_path: if integrate_workspace_changes {
                manager.write_through_workspace_root()
            } else {
                None
            },
            workspace_snapshot: if spec.input_context.include_workspace_snapshot {
                match manager
                    .workspace_snapshot(&worktree, cancellation.clone())
                    .await
                {
                    Ok(snapshot) => Some(snapshot),
                    Err(error) => {
                        return fail_worker(
                            &spec.worker_id,
                            &mut state,
                            "workspace_snapshot",
                            error.to_string(),
                            attempt,
                            last_worktree,
                            state_changes,
                        );
                    }
                }
            } else {
                None
            },
            input_artifacts: input_artifacts.clone(),
            cancellation: cancellation.clone(),
            control: control.clone(),
        };
        let execution = tokio::select! {
            _ = cancellation.cancelled() => Err(WorkerFailure::new("cancelled", "worker cancelled")),
            result = tokio::time::timeout(
                Duration::from_millis(spec.timeout_ms),
                executor.execute(context),
            ) => result.unwrap_or_else(|_| Err(WorkerFailure::new("timeout", "worker timed out"))),
        };
        match execution {
            Ok(output) => {
                if output.summary.trim().is_empty() {
                    return fail_worker(
                        &spec.worker_id,
                        &mut state,
                        "missing_output",
                        "worker returned an empty structured summary".into(),
                        attempt,
                        last_worktree,
                        state_changes,
                    );
                }
                if spec.role.eq_ignore_ascii_case("verifier") && output.verification.is_none() {
                    return fail_worker(
                        &spec.worker_id,
                        &mut state,
                        "missing_verification",
                        "Verifier did not return a VerificationRecord".into(),
                        attempt,
                        last_worktree,
                        state_changes,
                    );
                }
                let mut stored = Vec::new();
                for artifact in output.artifacts {
                    match manager.store_worker_artifact(
                        &spec.worker_id,
                        &artifact.name,
                        &artifact.media_type,
                        &artifact.bytes,
                    ) {
                        Ok(record) => stored.push(StoredWorkerArtifact {
                            record,
                            bytes: artifact.bytes,
                        }),
                        Err(error) => {
                            return fail_worker(
                                &spec.worker_id,
                                &mut state,
                                "artifact",
                                error.to_string(),
                                attempt,
                                last_worktree,
                                state_changes,
                            );
                        }
                    }
                }
                if stored.is_empty() {
                    let summary_bytes = output.summary.as_bytes().to_vec();
                    let record = match manager.store_worker_artifact(
                        &spec.worker_id,
                        "structured worker summary",
                        "text/plain; charset=utf-8",
                        &summary_bytes,
                    ) {
                        Ok(record) => record,
                        Err(error) => {
                            return fail_worker(
                                &spec.worker_id,
                                &mut state,
                                "artifact",
                                error.to_string(),
                                attempt,
                                last_worktree,
                                state_changes,
                            );
                        }
                    };
                    stored.push(StoredWorkerArtifact {
                        record,
                        bytes: summary_bytes,
                    });
                }
                let patch = match manager
                    .capture_patch(&worktree, &spec.write_scopes, cancellation.clone())
                    .await
                {
                    Ok(patch) => patch,
                    Err(error) => {
                        return fail_worker(
                            &spec.worker_id,
                            &mut state,
                            "patch",
                            error.to_string(),
                            attempt,
                            last_worktree,
                            state_changes,
                        );
                    }
                };
                if let Some(patch) = &patch {
                    stored.push(StoredWorkerArtifact {
                        record: patch.record.clone(),
                        bytes: patch.bytes.clone(),
                    });
                    if integrate_workspace_changes
                        && let Err(error) =
                            manager.integrate_patch(patch, cancellation.clone()).await
                    {
                        return fail_worker(
                            &spec.worker_id,
                            &mut state,
                            "integration",
                            error.to_string(),
                            attempt,
                            last_worktree,
                            state_changes,
                        );
                    }
                }
                if spec.role.eq_ignore_ascii_case("verifier")
                    && let Err(error) = transition(
                        &spec.worker_id,
                        &mut state,
                        WorkerState::Verifying,
                        "Verifier produced a structured verdict",
                        &mut state_changes,
                    )
                {
                    return failed_from_scheduler(error, attempt, last_worktree, state_changes);
                }
                if let Err(error) = transition(
                    &spec.worker_id,
                    &mut state,
                    WorkerState::Completed,
                    if patch.is_some() && integrate_workspace_changes {
                        "completion criteria produced artifacts and workspace changes were synchronized"
                    } else {
                        "completion criteria produced artifacts"
                    },
                    &mut state_changes,
                ) {
                    return failed_from_scheduler(error, attempt, last_worktree, state_changes);
                }
                return Ok(SuccessfulWorker {
                    attempts: attempt,
                    worktree,
                    artifacts: stored,
                    patch,
                    summary: output.summary,
                    verification: output.verification,
                    state_changes,
                });
            }
            Err(failure) => {
                let retryable = spec
                    .retry_policy
                    .retryable_error_codes
                    .iter()
                    .any(|code| code == &failure.code)
                    || (matches!(
                        failure.code.as_str(),
                        "tool_loop_limit" | "browser_acceptance_incomplete"
                    ) && !spec.write_scopes.is_empty());
                if retryable && attempt < spec.retry_policy.maximum_attempts {
                    if !spec.write_scopes.is_empty() {
                        match manager
                            .capture_patch(&worktree, &spec.write_scopes, cancellation.clone())
                            .await
                        {
                            Ok(Some(patch)) => resume_patches.push(patch),
                            Ok(None) => {}
                            Err(error) => {
                                return fail_worker(
                                    &spec.worker_id,
                                    &mut state,
                                    "retry_checkpoint",
                                    format!(
                                        "failed to checkpoint attempt {attempt} after {}: {error}",
                                        failure.code
                                    ),
                                    attempt,
                                    last_worktree,
                                    state_changes,
                                );
                            }
                        }
                    }
                    if let Err(error) = transition(
                        &spec.worker_id,
                        &mut state,
                        WorkerState::Paused,
                        format!(
                            "retryable failure {}: {}; workspace changes checkpointed for the next attempt",
                            failure.code, failure.message
                        ),
                        &mut state_changes,
                    ) {
                        return failed_from_scheduler(error, attempt, last_worktree, state_changes);
                    }
                    control.mark_state(&spec.worker_id, WorkerState::Paused);
                    previous_failure = Some(failure.clone());
                    tokio::select! {
                        _ = cancellation.cancelled() => {
                            return fail_worker(
                                &spec.worker_id,
                                &mut state,
                                "cancelled",
                                "cancelled during retry backoff".into(),
                                attempt,
                                last_worktree,
                                state_changes,
                            );
                        }
                        _ = tokio::time::sleep(Duration::from_millis(spec.retry_policy.backoff_ms)) => {}
                    }
                    continue;
                }
                return fail_worker(
                    &spec.worker_id,
                    &mut state,
                    &failure.code,
                    failure.message,
                    attempt,
                    last_worktree,
                    state_changes,
                );
            }
        }
    }
    fail_worker(
        &spec.worker_id,
        &mut state,
        "attempts_exhausted",
        "worker exhausted all attempts".into(),
        spec.retry_policy.maximum_attempts,
        last_worktree,
        state_changes,
    )
}

fn fail_worker(
    worker_id: &WorkerId,
    state: &mut WorkerState,
    code: &str,
    message: String,
    attempts: u32,
    worktree_path: Option<String>,
    mut state_changes: Vec<WorkerStateChange>,
) -> Result<SuccessfulWorker, FailedWorker> {
    let target = if code == "cancelled" {
        WorkerState::Cancelled
    } else {
        WorkerState::Failed
    };
    let _ = transition(
        worker_id,
        state,
        target,
        format!("{code}: {message}"),
        &mut state_changes,
    );
    Err(FailedWorker {
        attempts,
        worktree_path,
        code: code.into(),
        message,
        state_changes,
    })
}

fn failed_from_scheduler(
    error: SchedulerError,
    attempts: u32,
    worktree_path: Option<String>,
    state_changes: Vec<WorkerStateChange>,
) -> Result<SuccessfulWorker, FailedWorker> {
    Err(FailedWorker {
        attempts,
        worktree_path,
        code: "scheduler".into(),
        message: error.to_string(),
        state_changes,
    })
}

fn transition(
    worker_id: &WorkerId,
    state: &mut WorkerState,
    next: WorkerState,
    reason: impl Into<String>,
    changes: &mut Vec<WorkerStateChange>,
) -> Result<(), SchedulerError> {
    state
        .validate_transition(next)
        .map_err(|error| SchedulerError::InvalidTransition(error.to_string()))?;
    changes.push(WorkerStateChange {
        worker_id: worker_id.clone(),
        from: *state,
        to: next,
        reason: reason.into(),
    });
    *state = next;
    Ok(())
}

fn cancel_pending(
    pending: &BTreeSet<WorkerId>,
    records: &mut BTreeMap<WorkerId, WorkerExecutionRecord>,
    changes: &mut Vec<WorkerStateChange>,
    reason: &str,
) -> Result<(), SchedulerError> {
    for id in pending {
        let record = records.get_mut(id).ok_or(SchedulerError::WorkerMissing)?;
        transition(
            id,
            &mut record.state,
            WorkerState::Cancelled,
            reason,
            changes,
        )?;
        record.error_code = Some("cancelled".into());
    }
    Ok(())
}

fn dependency_artifacts(
    spec: &WorkerSpec,
    completed: &BTreeMap<WorkerId, CompletedWorker>,
) -> Vec<StoredWorkerArtifact> {
    spec.dependencies
        .iter()
        .filter_map(|dependency| completed.get(dependency))
        .flat_map(|worker| worker.artifacts.clone())
        .collect()
}

fn ancestor_patches(
    plan: &OrchestrationPlan,
    worker_id: &WorkerId,
    topological_order: &[WorkerId],
    patches: &BTreeMap<WorkerId, PatchArtifact>,
) -> Vec<PatchArtifact> {
    topological_order
        .iter()
        .filter(|candidate| depends_on(plan, worker_id, candidate))
        .filter_map(|candidate| patches.get(candidate).cloned())
        .collect()
}

fn depends_on(plan: &OrchestrationPlan, worker_id: &WorkerId, target: &WorkerId) -> bool {
    let by_id = plan
        .workers
        .iter()
        .map(|worker| (&worker.worker_id, worker))
        .collect::<BTreeMap<_, _>>();
    let mut pending = vec![worker_id];
    let mut seen = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        let Some(worker) = by_id.get(current) else {
            continue;
        };
        for dependency in &worker.dependencies {
            if dependency == target {
                return true;
            }
            pending.push(dependency);
        }
    }
    false
}

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("orchestration plan is invalid: {0:?}")]
    InvalidPlan(OrchestrationValidation),
    #[error("worker disappeared from the validated plan")]
    WorkerMissing,
    #[error("scheduler stalled with unresolved workers")]
    SchedulerStalled,
    #[error("invalid worker transition: {0}")]
    InvalidTransition(String),
    #[error(transparent)]
    Worktree(#[from] WorktreeError),
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        process::Command,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use lunascope_core::{
        AllowedWorkerModel, CriterionVerification, DelegationKind, OrchestrationPatch,
        OrchestrationPatchApplyMode, OrchestrationPatchOperation, VerificationStatus, WorkerPatch,
        WorkerState,
    };
    use tokio::sync::{Barrier, Notify};

    use super::*;
    use crate::draft_orchestration;

    #[test]
    fn verifier_must_cover_every_acceptance_criterion_with_direct_evidence() {
        let mut verification = VerificationRecord {
            status: VerificationStatus::Verified,
            summary: "all done".into(),
            evidence: vec!["global summary".into()],
            remaining_risks: Vec::new(),
            criterion_results: vec![CriterionVerification {
                criterion_id: "AC-1".into(),
                status: CriterionVerificationStatus::Passed,
                evidence: vec!["command A exited 0".into()],
                note: "observed".into(),
            }],
            findings: Vec::new(),
        };
        enforce_acceptance_coverage(
            &mut verification,
            &["first behavior".into(), "second behavior".into()],
        );
        assert_eq!(verification.status, VerificationStatus::FailedVerification);
        assert!(
            verification
                .findings
                .iter()
                .any(|finding| finding.title.contains("AC-2"))
        );

        verification.criterion_results.push(CriterionVerification {
            criterion_id: "AC-2".into(),
            status: CriterionVerificationStatus::Passed,
            evidence: vec!["browser interaction observed".into()],
            note: "observed".into(),
        });
        verification.status = VerificationStatus::Verified;
        verification.findings.clear();
        verification.remaining_risks.clear();
        enforce_acceptance_coverage(
            &mut verification,
            &["first behavior".into(), "second behavior".into()],
        );
        assert_eq!(verification.status, VerificationStatus::Verified);
        assert!(verification.findings.is_empty());
    }

    #[test]
    fn complete_criterion_ledger_overrides_generic_partial_risk() {
        let mut verification = VerificationRecord {
            status: VerificationStatus::PartiallyVerified,
            summary: "all acceptance checks passed".into(),
            evidence: vec!["browser and static checks passed".into()],
            remaining_risks: vec!["a generic environment caveat".into()],
            criterion_results: vec![CriterionVerification {
                criterion_id: "AC-1".into(),
                status: CriterionVerificationStatus::Passed,
                evidence: vec!["browser interaction observed".into()],
                note: "observed".into(),
            }],
            findings: Vec::new(),
        };

        enforce_acceptance_coverage(&mut verification, &["required behavior".into()]);

        assert_eq!(verification.status, VerificationStatus::Verified);
        assert!(verification.remaining_risks.is_empty());
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("git fixture");
        assert!(status.success(), "git fixture failed: {args:?}");
    }

    fn initialize_repository(root: &Path) {
        fs::create_dir_all(root).expect("repository");
        fs::write(root.join("README.md"), "base\n").expect("readme");
        run_git(root, &["init"]);
        run_git(root, &["config", "user.name", "LunaScope Test"]);
        run_git(root, &["config", "user.email", "lunascope@example.invalid"]);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "base"]);
    }

    fn prompt_revision(
        plan: &OrchestrationPlan,
        worker_id: &WorkerId,
        mode: OrchestrationPatchApplyMode,
        prompt: &str,
    ) -> (OrchestrationPatch, OrchestrationPlan) {
        let patch = OrchestrationPatch {
            patch_id: format!("patch-{}", Uuid::new_v4()),
            base_version: plan.version,
            apply_mode: mode,
            reason: "scheduler control test".into(),
            operations: vec![OrchestrationPatchOperation::UpdateWorker {
                patch: WorkerPatch {
                    worker_id: worker_id.clone(),
                    display_name: None,
                    role: None,
                    tags: None,
                    objective: None,
                    task: None,
                    prompt: Some(prompt.into()),
                    input_context: None,
                    expected_output: None,
                    output_schema: None,
                    completion_criteria: None,
                    owned_acceptance_criteria: None,
                    parallel_group: None,
                    model: None,
                    skills: None,
                    tools: None,
                    permissions: None,
                    budget: None,
                    dependencies: None,
                    timeout_ms: None,
                    retry_policy: None,
                    checkpoint_policy: None,
                    write_scopes: None,
                    parent_worker_id: None,
                    lock_fields: Vec::new(),
                    unlock_fields: Vec::new(),
                },
            }],
        };
        let revised = crate::apply_user_patch(plan, &patch).expect("revised plan");
        (patch, revised)
    }

    struct CollaborationExecutor {
        builders_started: Arc<Barrier>,
        visible_workspace: Option<PathBuf>,
    }

    struct RetryOnceExecutor {
        attempts: Arc<AtomicUsize>,
    }

    struct ResumeBudgetExecutor;

    struct AlwaysFailExecutor;

    impl WorkerExecutor for AlwaysFailExecutor {
        fn execute(&self, _context: WorkerExecutionContext) -> WorkerFuture {
            Box::pin(async { Err(WorkerFailure::new("fixture_failure", "root worker failed")) })
        }
    }

    impl WorkerExecutor for RetryOnceExecutor {
        fn execute(&self, _context: WorkerExecutionContext) -> WorkerFuture {
            let attempts = self.attempts.clone();
            Box::pin(async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(WorkerFailure::new(
                        "provider_unavailable",
                        "transient fixture outage",
                    ));
                }
                Ok(WorkerOutput {
                    summary: "retry completed with a structured result".into(),
                    artifacts: Vec::new(),
                    verification: None,
                })
            })
        }
    }

    impl WorkerExecutor for ResumeBudgetExecutor {
        fn execute(&self, context: WorkerExecutionContext) -> WorkerFuture {
            Box::pin(async move {
                let path = context.worktree_path.join("src/resume.txt");
                if context.attempt == 1 {
                    assert!(context.previous_failure.is_none());
                    fs::create_dir_all(path.parent().expect("parent"))
                        .map_err(|error| WorkerFailure::new("io", error.to_string()))?;
                    fs::write(&path, "first-attempt checkpoint\n")
                        .map_err(|error| WorkerFailure::new("io", error.to_string()))?;
                    return Err(WorkerFailure::new(
                        "tool_loop_limit",
                        "fixture exhausted its first execution window",
                    ));
                }
                let previous = context
                    .previous_failure
                    .as_ref()
                    .expect("retry receives its exact previous failure");
                assert_eq!(previous.code, "tool_loop_limit");
                assert!(previous.message.contains("first execution window"));
                let prior = fs::read_to_string(&path)
                    .map_err(|error| WorkerFailure::new("missing_checkpoint", error.to_string()))?;
                fs::write(&path, format!("{prior}second-attempt completion\n"))
                    .map_err(|error| WorkerFailure::new("io", error.to_string()))?;
                Ok(WorkerOutput {
                    summary: "resumed from the checkpoint and completed".into(),
                    artifacts: Vec::new(),
                    verification: None,
                })
            })
        }
    }

    struct BlockingExecutor {
        started: Arc<Notify>,
    }

    struct RetryRevisionExecutor {
        started: Arc<Notify>,
        release: Arc<Notify>,
        prompts: Arc<std::sync::Mutex<Vec<String>>>,
    }

    struct ContinuousRefillExecutor {
        child_started: Arc<Notify>,
    }

    struct DynamicTopologyExecutor {
        root_worker: WorkerId,
        removed_worker: WorkerId,
        replacement_worker: WorkerId,
        root_started: Arc<Notify>,
        release_root: Arc<Notify>,
        executed: Arc<std::sync::Mutex<Vec<WorkerId>>>,
    }

    impl WorkerExecutor for DynamicTopologyExecutor {
        fn execute(&self, context: WorkerExecutionContext) -> WorkerFuture {
            let root_worker = self.root_worker.clone();
            let removed_worker = self.removed_worker.clone();
            let replacement_worker = self.replacement_worker.clone();
            let root_started = self.root_started.clone();
            let release_root = self.release_root.clone();
            let executed = self.executed.clone();
            Box::pin(async move {
                let worker_id = context.spec.worker_id.clone();
                executed
                    .lock()
                    .expect("execution trace")
                    .push(worker_id.clone());
                if worker_id == removed_worker {
                    return Err(WorkerFailure::new(
                        "stale_topology_dispatched",
                        "the removed Worker must never execute",
                    ));
                }
                if worker_id == root_worker {
                    root_started.notify_one();
                    release_root.notified().await;
                }
                let summary = if worker_id == replacement_worker {
                    "replacement Worker completed the revised acceptance task"
                } else {
                    "original Worker completed without blocking the replan"
                };
                Ok(WorkerOutput {
                    summary: summary.into(),
                    artifacts: Vec::new(),
                    verification: None,
                })
            })
        }
    }

    impl WorkerExecutor for ContinuousRefillExecutor {
        fn execute(&self, context: WorkerExecutionContext) -> WorkerFuture {
            let child_started = self.child_started.clone();
            Box::pin(async move {
                match context.spec.role.as_str() {
                    "fast-root" => Ok(WorkerOutput {
                        summary: "fast root completed".into(),
                        artifacts: Vec::new(),
                        verification: None,
                    }),
                    "slow-root" => {
                        child_started.notified().await;
                        Ok(WorkerOutput {
                            summary: "slow root observed continuous refill".into(),
                            artifacts: Vec::new(),
                            verification: None,
                        })
                    }
                    "unlocked-child" => {
                        child_started.notify_one();
                        Ok(WorkerOutput {
                            summary: "child started as soon as its dependency completed".into(),
                            artifacts: Vec::new(),
                            verification: None,
                        })
                    }
                    role => Err(WorkerFailure::new(
                        "unexpected_role",
                        format!("unexpected refill fixture role {role}"),
                    )),
                }
            })
        }
    }

    impl WorkerExecutor for RetryRevisionExecutor {
        fn execute(&self, context: WorkerExecutionContext) -> WorkerFuture {
            let started = self.started.clone();
            let release = self.release.clone();
            let prompts = self.prompts.clone();
            Box::pin(async move {
                prompts
                    .lock()
                    .expect("prompts")
                    .push(context.spec.prompt.clone());
                if context.attempt == 1 {
                    started.notify_one();
                    release.notified().await;
                    return Err(WorkerFailure::new(
                        "provider_unavailable",
                        "fixture retry boundary",
                    ));
                }
                Ok(WorkerOutput {
                    summary: context.spec.prompt,
                    artifacts: Vec::new(),
                    verification: None,
                })
            })
        }
    }

    impl WorkerExecutor for BlockingExecutor {
        fn execute(&self, _context: WorkerExecutionContext) -> WorkerFuture {
            let started = self.started.clone();
            Box::pin(async move {
                started.notify_one();
                std::future::pending::<Result<WorkerOutput, WorkerFailure>>().await
            })
        }
    }

    impl WorkerExecutor for CollaborationExecutor {
        fn execute(&self, context: WorkerExecutionContext) -> WorkerFuture {
            let barrier = self.builders_started.clone();
            let visible_workspace = self.visible_workspace.clone();
            Box::pin(async move {
                match context.spec.role.as_str() {
                    "builder-a" | "builder-b" => {
                        barrier.wait().await;
                        let name = if context.spec.role == "builder-a" {
                            "a.txt"
                        } else {
                            "b.txt"
                        };
                        fs::create_dir_all(context.worktree_path.join("src"))
                            .map_err(|error| WorkerFailure::new("io", error.to_string()))?;
                        fs::write(
                            context.worktree_path.join("src").join(name),
                            format!("produced by {}\n", context.spec.role),
                        )
                        .map_err(|error| WorkerFailure::new("io", error.to_string()))?;
                        Ok(WorkerOutput {
                            summary: format!("{} produced {name}", context.spec.role),
                            artifacts: vec![ProducedWorkerArtifact {
                                name: format!("{} report", context.spec.role),
                                media_type: "application/json".into(),
                                bytes: format!(
                                    "{{\"worker\":\"{}\",\"path\":\"src/{name}\"}}",
                                    context.spec.role
                                )
                                .into_bytes(),
                            }],
                            verification: None,
                        })
                    }
                    "verifier" => {
                        if let Some(workspace) = visible_workspace {
                            for path in ["src/a.txt", "src/b.txt"] {
                                if !workspace.join(path).is_file() {
                                    return Err(WorkerFailure::new(
                                        "workspace_visibility",
                                        format!(
                                            "{path} was not synchronized before verifier dispatch"
                                        ),
                                    ));
                                }
                            }
                        }
                        if context.input_artifacts.len() < 4 {
                            return Err(WorkerFailure::new(
                                "handoff",
                                "expected artifacts and patches from both builders",
                            ));
                        }
                        let a = fs::read_to_string(context.worktree_path.join("src/a.txt"))
                            .map_err(|error| WorkerFailure::new("verify", error.to_string()))?;
                        let b = fs::read_to_string(context.worktree_path.join("src/b.txt"))
                            .map_err(|error| WorkerFailure::new("verify", error.to_string()))?;
                        if !a.contains("builder-a") || !b.contains("builder-b") {
                            return Err(WorkerFailure::new(
                                "verify",
                                "integrated content is incorrect",
                            ));
                        }
                        Ok(WorkerOutput {
                            summary:
                                "Verifier observed both isolated patches and handoff artifacts."
                                    .into(),
                            artifacts: Vec::new(),
                            verification: Some(VerificationRecord {
                                status: VerificationStatus::Verified,
                                summary: "Both disjoint patches were integrated and inspected."
                                    .into(),
                                evidence: vec!["src/a.txt".into(), "src/b.txt".into()],
                                remaining_risks: Vec::new(),
                                criterion_results: Vec::new(),
                                findings: Vec::new(),
                            }),
                        })
                    }
                    role => Err(WorkerFailure::new(
                        "unsupported_role",
                        format!("unexpected fixture role {role}"),
                    )),
                }
            })
        }
    }

    #[tokio::test]
    async fn two_workers_run_in_parallel_and_handoff_patches_to_verifier() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let mut plan = draft_orchestration(
            "Research and implement multiple modules, then verify",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        assert_eq!(plan.decision.kind, DelegationKind::MultiAgent);
        let a = plan.workers[0].worker_id.clone();
        let b = plan.workers[1].worker_id.clone();
        plan.workers[0].role = "builder-a".into();
        plan.workers[0].dependencies.clear();
        plan.workers[0].write_scopes = vec!["src/a.txt".into()];
        plan.workers[1].role = "builder-b".into();
        plan.workers[1].dependencies.clear();
        plan.workers[1].write_scopes = vec!["src/b.txt".into()];
        plan.workers[2].role = "verifier".into();
        plan.workers[2].dependencies = vec![a, b];
        plan.workers[2].write_scopes.clear();

        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let scheduler = AgentScheduler::new(manager);
        let result = scheduler
            .run(
                &plan,
                Arc::new(CollaborationExecutor {
                    builders_started: Arc::new(Barrier::new(2)),
                    visible_workspace: None,
                }),
                CancellationToken::new(),
            )
            .await
            .expect("run");
        assert_eq!(result.verification.status, VerificationStatus::Verified);
        assert_eq!(result.handoffs.len(), 2);
        assert!(
            result
                .workers
                .values()
                .all(|record| record.state == WorkerState::Completed)
        );
        assert!(result.artifacts.len() >= 5);
        assert!(result.synthesis.contains("builder-a"));
        assert!(result.synthesis.contains("Verifier observed"));
        assert!(!repository.join("src/a.txt").exists());
        assert!(!repository.join("src/b.txt").exists());
    }

    #[tokio::test]
    async fn scheduler_refills_a_freed_slot_without_waiting_for_the_batch() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let mut plan = draft_orchestration(
            "Research and implement multiple modules, then verify",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        let fast = plan.workers[0].worker_id.clone();
        plan.maximum_parallel_workers = 2;
        plan.workers[0].role = "fast-root".into();
        plan.workers[0].dependencies.clear();
        plan.workers[0].write_scopes.clear();
        plan.workers[1].role = "slow-root".into();
        plan.workers[1].dependencies.clear();
        plan.workers[1].write_scopes.clear();
        plan.workers[2].role = "unlocked-child".into();
        plan.workers[2].dependencies = vec![fast];
        plan.workers[2].write_scopes.clear();
        let child_started = Arc::new(Notify::new());
        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            AgentScheduler::new(manager).run(
                &plan,
                Arc::new(ContinuousRefillExecutor { child_started }),
                CancellationToken::new(),
            ),
        )
        .await
        .expect("continuous-refill scheduler did not stall")
        .expect("run");

        assert!(
            result
                .workers
                .values()
                .all(|record| record.state == WorkerState::Completed)
        );
        assert!(result.synthesis.contains("continuous refill"));
    }

    #[tokio::test]
    async fn scheduler_applies_a_live_topology_replan_without_restarting_active_work() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let mut plan = draft_orchestration(
            "Research and implement multiple modules, then verify",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        plan.maximum_parallel_workers = 2;
        let root_worker = plan.workers[0].worker_id.clone();
        let removed = plan.workers.last().expect("replaceable leaf").clone();
        plan.workers[0].role = "dynamic-root".into();
        plan.workers[0].dependencies.clear();
        plan.workers[0].write_scopes.clear();
        plan.workers[1].role = "independent-sibling".into();
        plan.workers[1].dependencies.clear();
        plan.workers[1].write_scopes.clear();
        plan.workers[2].dependencies = vec![root_worker.clone(), plan.workers[1].worker_id.clone()];
        plan.workers[2].write_scopes.clear();

        let mut replacement = plan.workers[2].clone();
        replacement.worker_id = WorkerId::new(format!("worker-{}", Uuid::new_v4()));
        replacement.display_name = "Verify the revised module boundary".into();
        replacement.role = "reviewer".into();
        replacement.task =
            "Verify only the newly guided module boundary and preserve completed work.".into();
        let patch = OrchestrationPatch {
            patch_id: format!("patch-{}", Uuid::new_v4()),
            base_version: plan.version,
            apply_mode: OrchestrationPatchApplyMode::ApplyAfterCurrentStep,
            reason: "user guidance narrows the unfinished verification task".into(),
            operations: vec![
                OrchestrationPatchOperation::RemoveWorker {
                    worker_id: removed.worker_id.clone(),
                },
                OrchestrationPatchOperation::AddWorker {
                    spec: replacement.clone(),
                },
            ],
        };
        let revised = crate::apply_user_patch(&plan, &patch).expect("revised topology");
        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let control = Arc::new(SchedulerControl::new(&plan));
        let root_started = Arc::new(Notify::new());
        let release_root = Arc::new(Notify::new());
        let executed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let scheduler_task = {
            let control = control.clone();
            let executor = Arc::new(DynamicTopologyExecutor {
                root_worker: root_worker.clone(),
                removed_worker: removed.worker_id.clone(),
                replacement_worker: replacement.worker_id.clone(),
                root_started: root_started.clone(),
                release_root: release_root.clone(),
                executed: executed.clone(),
            });
            let plan = plan.clone();
            tokio::spawn(async move {
                AgentScheduler::new(manager)
                    .run_controlled(&plan, executor, CancellationToken::new(), control)
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(30), root_started.notified())
            .await
            .expect("dynamic root did not start");
        control
            .submit_revision(&patch, &revised)
            .expect("live replan accepted while the root remains active");
        release_root.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(30), scheduler_task)
            .await
            .expect("dynamic scheduler did not stall")
            .expect("join")
            .expect("run");

        let executed = executed.lock().expect("execution trace");
        assert!(executed.contains(&root_worker));
        assert!(executed.contains(&replacement.worker_id));
        assert!(!executed.contains(&removed.worker_id));
        assert_eq!(result.version, revised.version);
        assert_eq!(result.workers[&root_worker].attempts, 1);
        assert_eq!(result.workers[&root_worker].state, WorkerState::Completed);
        assert_eq!(
            result.workers[&removed.worker_id].state,
            WorkerState::Cancelled
        );
        assert_eq!(
            result.workers[&replacement.worker_id].state,
            WorkerState::Completed
        );
    }

    #[tokio::test]
    async fn integrating_scheduler_synchronizes_each_completed_node_before_verifier_dispatch() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let mut plan = draft_orchestration(
            "Research and implement multiple modules, then verify",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        let a = plan.workers[0].worker_id.clone();
        let b = plan.workers[1].worker_id.clone();
        plan.workers[0].role = "builder-a".into();
        plan.workers[0].dependencies.clear();
        plan.workers[0].write_scopes = vec!["src/a.txt".into()];
        plan.workers[1].role = "builder-b".into();
        plan.workers[1].dependencies.clear();
        plan.workers[1].write_scopes = vec!["src/b.txt".into()];
        plan.workers[2].role = "verifier".into();
        plan.workers[2].dependencies = vec![a, b];
        plan.workers[2].write_scopes.clear();

        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let scheduler = AgentScheduler::new_integrating(manager);
        let result = scheduler
            .run(
                &plan,
                Arc::new(CollaborationExecutor {
                    builders_started: Arc::new(Barrier::new(2)),
                    visible_workspace: Some(repository.clone()),
                }),
                CancellationToken::new(),
            )
            .await
            .expect("run");

        assert_eq!(result.verification.status, VerificationStatus::Verified);
        assert_eq!(
            fs::read_to_string(repository.join("src/a.txt")).expect("a.txt"),
            "produced by builder-a\n"
        );
        assert_eq!(
            fs::read_to_string(repository.join("src/b.txt")).expect("b.txt"),
            "produced by builder-b\n"
        );
    }

    #[tokio::test]
    async fn retryable_provider_failure_uses_a_fresh_attempt_and_completes() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let mut plan = draft_orchestration(
            "Short answer: summarize the README",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        assert_eq!(plan.decision.kind, DelegationKind::SingleAgent);
        plan.workers[0].retry_policy.backoff_ms = 1;
        let worker_id = plan.workers[0].worker_id.clone();
        let attempts = Arc::new(AtomicUsize::new(0));
        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let result = AgentScheduler::new(manager)
            .run(
                &plan,
                Arc::new(RetryOnceExecutor {
                    attempts: attempts.clone(),
                }),
                CancellationToken::new(),
            )
            .await
            .expect("run");

        let record = result.workers.get(&worker_id).expect("record");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(record.attempts, 2);
        assert_eq!(record.state, WorkerState::Completed);
        assert!(
            result.state_changes.iter().any(|change| {
                change.worker_id == worker_id && change.to == WorkerState::Paused
            })
        );
    }

    #[tokio::test]
    async fn tool_budget_retry_resumes_from_the_previous_attempt_checkpoint() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let mut plan = draft_orchestration(
            "Create src/resume.txt",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        plan.workers[0].dependencies.clear();
        plan.workers[0].write_scopes = vec!["src".into()];
        plan.workers[0].retry_policy.maximum_attempts = 2;
        plan.workers[0].retry_policy.backoff_ms = 1;
        let worker_id = plan.workers[0].worker_id.clone();
        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");

        let result = AgentScheduler::new_integrating(manager)
            .run(
                &plan,
                Arc::new(ResumeBudgetExecutor),
                CancellationToken::new(),
            )
            .await
            .expect("run");

        let record = result.workers.get(&worker_id).expect("record");
        assert_eq!(record.state, WorkerState::Completed);
        assert_eq!(record.attempts, 2);
        assert_eq!(
            fs::read_to_string(repository.join("src/resume.txt")).expect("integrated file"),
            "first-attempt checkpoint\nsecond-attempt completion\n"
        );
        assert!(result.state_changes.iter().any(|change| {
            change.worker_id == worker_id
                && change.to == WorkerState::Paused
                && change.reason.contains("checkpointed")
        }));
    }

    #[tokio::test]
    async fn dependency_failure_propagates_through_all_descendants_without_stalling() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let mut plan = draft_orchestration(
            "Research the runtime, implement the result, and independently verify it",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        assert!(plan.workers.len() >= 3);
        let root = plan.workers[0].worker_id.clone();
        let child = plan.workers[1].worker_id.clone();
        let descendant = plan.workers[2].worker_id.clone();
        plan.workers[0].dependencies.clear();
        plan.workers[0].retry_policy.maximum_attempts = 1;
        plan.workers[1].dependencies = vec![root.clone()];
        plan.workers[2].dependencies = vec![child.clone()];

        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let result = AgentScheduler::new(manager)
            .run(
                &plan,
                Arc::new(AlwaysFailExecutor),
                CancellationToken::new(),
            )
            .await
            .expect("dependency failures should return a terminal result");

        assert_eq!(result.workers[&root].state, WorkerState::Failed);
        assert_eq!(result.workers[&child].state, WorkerState::Cancelled);
        assert_eq!(
            result.workers[&child].error_code.as_deref(),
            Some("dependency_failed")
        );
        assert_eq!(result.workers[&descendant].state, WorkerState::Cancelled);
        assert_eq!(
            result.workers[&descendant].error_code.as_deref(),
            Some("dependency_failed")
        );
        assert_eq!(
            result.verification.status,
            VerificationStatus::FailedVerification
        );
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_running_worker_and_records_terminal_state() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let plan = draft_orchestration(
            "Short answer: summarize the README",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        let worker_id = plan.workers[0].worker_id.clone();
        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let cancellation = CancellationToken::new();
        let started = Arc::new(Notify::new());
        let cancellation_task = {
            let cancellation = cancellation.clone();
            let started = started.clone();
            tokio::spawn(async move {
                started.notified().await;
                cancellation.cancel();
            })
        };
        let result = AgentScheduler::new(manager)
            .run(&plan, Arc::new(BlockingExecutor { started }), cancellation)
            .await
            .expect("run");
        cancellation_task.await.expect("cancellation task");

        let record = result.workers.get(&worker_id).expect("record");
        assert_eq!(record.state, WorkerState::Cancelled);
        assert_eq!(record.error_code.as_deref(), Some("cancelled"));
        assert_eq!(
            result.verification.status,
            VerificationStatus::UnableToVerify
        );
        assert!(result.verification.findings.is_empty());
        assert!(
            result
                .workers
                .values()
                .all(|worker| worker.state != WorkerState::Failed)
        );
        assert!(
            !result
                .state_changes
                .iter()
                .any(|change| change.to == WorkerState::Failed)
        );
        assert!(result.state_changes.iter().any(|change| {
            change.worker_id == worker_id && change.to == WorkerState::Cancelled
        }));
    }

    #[test]
    fn live_dispatch_modes_update_only_workers_that_have_not_started() {
        let plan = draft_orchestration(
            "Research the runtime and independently verify it",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        let worker_id = plan.workers[1].worker_id.clone();
        let control = SchedulerControl::new(&plan);
        control.mark_state(&worker_id, WorkerState::WaitingDependency);
        let (after_patch, after_plan) = prompt_revision(
            &plan,
            &worker_id,
            OrchestrationPatchApplyMode::ApplyAfterCurrentStep,
            "AFTER STEP PROMPT",
        );
        control
            .submit_revision(&after_patch, &after_plan)
            .expect("after-step revision");
        assert_eq!(
            control
                .take_dispatch_revision(&worker_id)
                .expect("dispatch revision")
                .spec
                .prompt,
            "AFTER STEP PROMPT"
        );

        control.mark_state(&worker_id, WorkerState::RunningModel);
        let (now_patch, now_plan) = prompt_revision(
            &after_plan,
            &worker_id,
            OrchestrationPatchApplyMode::ApplyNow,
            "NOW PROMPT",
        );
        assert!(matches!(
            control.submit_revision(&now_patch, &now_plan),
            Err(SchedulerControlError::InvalidWorkerState { .. })
        ));
    }

    #[test]
    fn live_topology_revision_replaces_only_not_started_workers() {
        let plan = draft_orchestration(
            "Research the runtime and independently verify it",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        let removed = plan.workers.last().expect("leaf Worker").clone();
        let mut replacement = removed.clone();
        replacement.worker_id = WorkerId::new(format!("worker-{}", Uuid::new_v4()));
        replacement.display_name = "Verify revised acceptance scope".into();
        let patch = OrchestrationPatch {
            patch_id: format!("patch-{}", Uuid::new_v4()),
            base_version: plan.version,
            apply_mode: OrchestrationPatchApplyMode::ApplyAfterCurrentStep,
            reason: "guidance requires a narrower verifier".into(),
            operations: vec![
                OrchestrationPatchOperation::RemoveWorker {
                    worker_id: removed.worker_id.clone(),
                },
                OrchestrationPatchOperation::AddWorker {
                    spec: replacement.clone(),
                },
            ],
        };
        let revised = crate::apply_user_patch(&plan, &patch).expect("revised topology");
        let control = SchedulerControl::new(&plan);
        control.mark_state(&removed.worker_id, WorkerState::WaitingDependency);
        control
            .submit_revision(&patch, &revised)
            .expect("live topology accepted");

        assert_eq!(
            control.worker_state(&removed.worker_id),
            Some(WorkerState::Cancelled)
        );
        assert_eq!(
            control.worker_state(&replacement.worker_id),
            Some(WorkerState::Draft)
        );
        assert_eq!(
            control
                .take_topology_revision()
                .expect("pending topology")
                .version,
            revised.version
        );
    }

    #[tokio::test]
    async fn apply_on_retry_changes_the_second_provider_invocation() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        initialize_repository(&repository);
        let mut plan = draft_orchestration(
            "Short answer: summarize the README",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        plan.workers[0].retry_policy.backoff_ms = 10;
        let worker_id = plan.workers[0].worker_id.clone();
        let manager = WorktreeManager::open(
            &repository,
            temporary.path().join("data"),
            &plan.orchestration_id,
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let control = Arc::new(SchedulerControl::new(&plan));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let scheduler_task = {
            let plan = plan.clone();
            let control = control.clone();
            let executor = Arc::new(RetryRevisionExecutor {
                started: started.clone(),
                release: release.clone(),
                prompts: prompts.clone(),
            });
            tokio::spawn(async move {
                AgentScheduler::new(manager)
                    .run_controlled(&plan, executor, CancellationToken::new(), control)
                    .await
            })
        };
        started.notified().await;
        let (patch, revised) = prompt_revision(
            &plan,
            &worker_id,
            OrchestrationPatchApplyMode::ApplyOnRetry,
            "REVISED RETRY PROMPT",
        );
        control
            .submit_revision(&patch, &revised)
            .expect("retry revision");
        release.notify_one();
        let result = scheduler_task.await.expect("join").expect("run");
        let prompts = prompts.lock().expect("prompts");
        assert_eq!(prompts.len(), 2);
        assert_ne!(prompts[0], "REVISED RETRY PROMPT");
        assert_eq!(prompts[1], "REVISED RETRY PROMPT");
        assert_eq!(result.version, 2);
        assert_eq!(result.workers[&worker_id].attempts, 2);
    }

    #[test]
    fn live_guidance_is_targeted_and_consumed_once() {
        let plan = draft_orchestration(
            "Research the runtime and independently verify it",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        let target = plan.workers[0].worker_id.clone();
        let untouched = plan.workers[1].worker_id.clone();
        let control = SchedulerControl::new(&plan);

        control
            .submit_guidance(
                [target.clone()],
                "Preserve the existing API boundary".into(),
            )
            .expect("queue guidance");

        assert_eq!(
            control.take_guidance(&target),
            vec!["Preserve the existing API boundary"]
        );
        assert!(control.take_guidance(&target).is_empty());
        assert!(control.take_guidance(&untouched).is_empty());
    }

    #[tokio::test]
    async fn pause_blocks_new_safe_steps_until_resume() {
        let plan = draft_orchestration(
            "Short answer: summarize the README",
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
            ],
        )
        .expect("plan");
        let control = Arc::new(SchedulerControl::new(&plan));
        control.pause().expect("pause scheduler");
        let waiter = {
            let control = control.clone();
            tokio::spawn(async move { control.wait_if_paused(&CancellationToken::new()).await })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !waiter.is_finished(),
            "paused scheduler must not dispatch a new step"
        );
        control.resume().expect("resume scheduler");
        assert!(
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .expect("waiter resumed before timeout")
                .expect("waiter task")
        );
    }
}
