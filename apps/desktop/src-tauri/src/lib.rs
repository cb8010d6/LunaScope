use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use include_dir::{Dir, include_dir};
use lunascope_core::{
    CorrelationId, CredentialReference, EventData, EventEnvelope, EventId, EventSource,
    GithubImportPreview, ImportComparison, ImportInstallRequest, InstalledImportVersion,
    LoadedSkill, McpConfigValue, McpInvocationResult, McpServerConfig, McpServerStatus,
    McpTransportConfig, ModelRoutingPolicy, ModelSelectionSettings, NaturalLanguageRoutingDraft,
    PermissionContext, PermissionKind, PermissionRequest, PolicyDecision, ProjectId,
    ProviderConfig, ProviderType, RiskLevel, RunId, RuntimeDelta, RuntimeSnapshot, SkillSourceKind,
    SkillSummary, ThreadId, ToolManifest, ToolRoutingDecision, ToolRoutingRequest,
};
use lunascope_extensions::{
    GithubImportManager, McpCredentialStore, McpSecretValue, NativeMcpClient, SkillCatalog,
    SkillRoot, validate_mcp_config,
};
use lunascope_integrations::{
    KeyringCredentialStore, NativeProviderClient, ProviderInvocation, ProviderMessage,
    ProviderMessageRole, SecretValue, parse_routing_preference, validate_model_selection_settings,
    validate_provider_config, validate_routing_policy,
};
use lunascope_runtime::{PolicyEngine, ToolRouter};
use lunascope_storage::SqliteEventStore;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State, ipc::Channel};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

mod attachment;
mod companion;
mod harness_prompt;
mod orchestration;
mod project;
mod ultranote;

static BUILTIN_SYSTEM_SKILLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/resources/system-skills");
static EXTENSION_DIRECTORY_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct AppState {
    pub(crate) store: Arc<SqliteEventStore>,
    pub(crate) credentials: KeyringCredentialStore,
    mcp_credentials: McpCredentialStore,
    pub(crate) active_orchestration: Mutex<Option<orchestration::ActiveOrchestrationInvocation>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRunRequest {
    run_id: String,
    title: String,
    initial_prompt: String,
    project_id: Option<String>,
    thread_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
enum RuntimeMessage {
    Snapshot(Box<RuntimeSnapshot>),
    Delta(RuntimeDelta),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderConnectionResult {
    response_id: String,
    text: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionDirectories {
    root: String,
    system_skills: String,
    user_skills: String,
    tools: String,
    mcp: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpCredentialInput {
    reference_id: String,
    secret: String,
}

#[tauri::command]
fn create_run(
    state: State<'_, AppState>,
    request: CreateRunRequest,
) -> Result<RuntimeSnapshot, String> {
    let run_id = RunId::new(request.run_id);
    if let Some(snapshot) = state.store.projection(&run_id).map_err(display_error)? {
        return Ok(snapshot);
    }

    let event = EventEnvelope::new(
        EventId::new(Uuid::new_v4().to_string()),
        1,
        jiff::Timestamp::now().to_string(),
        ProjectId::from(
            request
                .project_id
                .unwrap_or_else(|| "lunascope-desktop".into()),
        ),
        ThreadId::new(
            request
                .thread_id
                .unwrap_or_else(|| format!("thread-{}", run_id.as_str())),
        ),
        run_id.clone(),
        CorrelationId::new(Uuid::new_v4().to_string()),
        EventSource::User,
        EventData::RunCreated {
            title: request.title,
            initial_prompt: request.initial_prompt,
        },
    );
    state.store.append(&event).map_err(display_error)?;
    state
        .store
        .projection(&run_id)
        .map_err(display_error)?
        .ok_or_else(|| "run projection missing after committed event".to_owned())
}

#[tauri::command]
fn delete_conversation(state: State<'_, AppState>, thread_id: String) -> Result<usize, String> {
    state.store.delete_thread(&thread_id).map_err(display_error)
}

#[tauri::command]
fn get_runtime_snapshot(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<RuntimeSnapshot>, String> {
    state
        .store
        .recover(&RunId::new(run_id))
        .map_err(display_error)
}

#[tauri::command]
fn subscribe_run(
    state: State<'_, AppState>,
    run_id: String,
    after_sequence: u64,
    on_message: Channel<RuntimeMessage>,
) -> Result<(), String> {
    let run_id = RunId::new(run_id);
    let snapshot = state
        .store
        .recover(&run_id)
        .map_err(display_error)?
        .ok_or_else(|| format!("run does not exist: {run_id}"))?;

    if after_sequence == 0 || after_sequence > snapshot.sequence {
        on_message
            .send(RuntimeMessage::Snapshot(Box::new(snapshot)))
            .map_err(display_error)?;
        return Ok(());
    }

    let events = state
        .store
        .events_after(&run_id, after_sequence)
        .map_err(display_error)?;
    if !events.is_empty() {
        let to_sequence = events
            .last()
            .expect("non-empty event list has a last event")
            .sequence;
        on_message
            .send(RuntimeMessage::Delta(RuntimeDelta {
                from_sequence: after_sequence,
                to_sequence,
                events,
            }))
            .map_err(display_error)?;
    }
    Ok(())
}

#[tauri::command]
fn list_provider_configs(state: State<'_, AppState>) -> Result<Vec<ProviderConfig>, String> {
    state.store.provider_configs().map_err(display_error)
}

#[tauri::command]
fn save_provider_config(
    state: State<'_, AppState>,
    config: ProviderConfig,
    credential: Option<String>,
) -> Result<(), String> {
    validate_provider_config(&config).map_err(display_error)?;
    let credential_rollback = if let Some(credential) = credential {
        let reference = CredentialReference {
            id: config.credential_reference_id.clone(),
            provider: config.provider_type,
            label: config.display_name.clone(),
        };
        let previous_secret = state
            .credentials
            .get_optional(&reference)
            .map_err(display_error)?;
        let secret = SecretValue::new(credential).map_err(display_error)?;
        state
            .credentials
            .put(&reference, &secret)
            .map_err(display_error)?;
        Some((reference, previous_secret))
    } else {
        None
    };
    if let Err(error) = state.store.save_provider_config(&config) {
        if let Some((reference, previous_secret)) = credential_rollback {
            let rollback = match previous_secret {
                Some(secret) => state.credentials.put(&reference, &secret),
                None => state.credentials.delete(&reference),
            };
            return Err(match rollback {
                Ok(()) => format!(
                    "provider metadata was not saved; the previous credential state was restored: {error}"
                ),
                Err(rollback_error) => format!(
                    "provider metadata was not saved and credential rollback failed: {error}; {rollback_error}"
                ),
            });
        }
        return Err(display_error(error));
    }
    Ok(())
}

#[tauri::command]
fn delete_provider_credential(
    state: State<'_, AppState>,
    reference: CredentialReference,
) -> Result<(), String> {
    state.credentials.delete(&reference).map_err(display_error)
}

#[tauri::command]
async fn test_provider_connection(
    state: State<'_, AppState>,
    provider_config_id: String,
    model: String,
) -> Result<ProviderConnectionResult, String> {
    let config = state
        .store
        .provider_configs()
        .map_err(display_error)?
        .into_iter()
        .find(|config| config.id == provider_config_id)
        .ok_or_else(|| format!("provider configuration not found: {provider_config_id}"))?;
    let client =
        NativeProviderClient::from_keyring(&config, &state.credentials).map_err(display_error)?;
    let summary = client
        .stream(
            &ProviderInvocation {
                model,
                instructions: Some("This is a connection test. Reply with exactly: OK".into()),
                messages: vec![ProviderMessage {
                    role: ProviderMessageRole::User,
                    content: serde_json::json!("Connection test"),
                }],
                tools: Vec::new(),
                max_output_tokens: 16,
                thinking_enabled: (config.provider_type == lunascope_core::ProviderType::DeepSeek)
                    .then_some(false),
                reasoning_effort: None,
                reasoning_summary: None,
            },
            std::time::Duration::from_secs(30),
            CancellationToken::new(),
            |_| Ok(()),
        )
        .await
        .map_err(display_error)?;
    Ok(ProviderConnectionResult {
        response_id: summary.response_id,
        text: summary.text,
        input_tokens: summary.usage.input_tokens,
        output_tokens: summary.usage.output_tokens,
    })
}

#[tauri::command]
fn get_routing_policy(
    state: State<'_, AppState>,
    scope_id: String,
) -> Result<Option<ModelRoutingPolicy>, String> {
    state.store.routing_policy(&scope_id).map_err(display_error)
}

#[tauri::command]
fn save_routing_policy(
    state: State<'_, AppState>,
    scope_id: String,
    policy: ModelRoutingPolicy,
) -> Result<(), String> {
    validate_routing_policy(&policy).map_err(display_error)?;
    state
        .store
        .save_routing_policy(&scope_id, &policy)
        .map_err(display_error)
}

#[tauri::command]
async fn parse_model_preference(
    state: State<'_, AppState>,
    source: String,
    base_policy: ModelRoutingPolicy,
    allow_once: bool,
) -> Result<NaturalLanguageRoutingDraft, String> {
    parse_model_preference_with_state(&state, source, base_policy, allow_once).await
}

async fn parse_model_preference_with_state(
    state: &AppState,
    source: String,
    base_policy: ModelRoutingPolicy,
    allow_once: bool,
) -> Result<NaturalLanguageRoutingDraft, String> {
    if source.trim().is_empty() {
        return Err("routing preference is required".to_owned());
    }
    let providers = state.store.provider_configs().map_err(display_error)?;
    let (provider, model) = orchestration::selected_orchestration_model(state, &providers)?;
    enforce_permissions(
        &[
            PermissionRequest {
                permission: PermissionKind::NetworkConnect,
                context: PermissionContext {
                    workspace_root: data_root()?.to_string_lossy().into_owned(),
                    network_domain: url::Url::parse(&provider.base_url)
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_owned)),
                    tool_id: Some("routing.preference.parse".to_owned()),
                    ..PermissionContext::default()
                },
                risk: RiskLevel::Medium,
                action:
                    "interpret a natural-language routing preference with the Orchestration model"
                        .to_owned(),
            },
            PermissionRequest {
                permission: PermissionKind::SecretsUse,
                context: PermissionContext {
                    workspace_root: data_root()?.to_string_lossy().into_owned(),
                    tool_id: Some("routing.preference.parse".to_owned()),
                    ..PermissionContext::default()
                },
                risk: RiskLevel::High,
                action: "use the configured Orchestration provider credential".to_owned(),
            },
        ],
        allow_once,
    )?;
    let client =
        NativeProviderClient::from_keyring(&provider, &state.credentials).map_err(display_error)?;
    let prompt = format!(
        "User routing preference:\n{}\n\nExisting policy JSON (preserve fields the user did not change):\n{}",
        source.trim(),
        serde_json::to_string_pretty(&base_policy).map_err(display_error)?
    );
    let parsed = client
        .stream(
            &ProviderInvocation {
                model,
                instructions: Some(
                    "Interpret the user's natural-language model-routing preference. Return JSON only, shaped exactly as NaturalLanguageRoutingDraft: sourceText, policy, matchedRules, warnings, confirmed. Preserve every existing policy field that the user did not explicitly change. Use provider configuration IDs only for allowed/disallowed ID lists; use open_ai, anthropic, deep_seek, compatible only in rolePreferences.preferredProviders. Never turn a soft preference into a hardConstraint unless the user explicitly says only, must, required, or equivalent. confirmed must be false. matchedRules must concisely state each applied change."
                        .to_owned(),
                ),
                messages: vec![ProviderMessage {
                    role: ProviderMessageRole::User,
                    content: serde_json::json!(prompt),
                }],
                tools: Vec::new(),
                max_output_tokens: 3_500,
                thinking_enabled: (provider.provider_type == ProviderType::DeepSeek)
                    .then_some(false),
                reasoning_effort: Some("high".to_owned()),
                reasoning_summary: None,
            },
            std::time::Duration::from_secs(60),
            CancellationToken::new(),
            |_| Ok(()),
        )
        .await;
    match parsed {
        Ok(summary) => {
            let json = orchestration::strip_json_fence(summary.text.trim());
            let json = orchestration::extract_first_json_object(json).unwrap_or(json);
            let mut draft = parse_model_routing_draft(json)?;
            draft.source_text = source;
            draft.confirmed = false;
            validate_routing_policy(&draft.policy).map_err(display_error)?;
            Ok(draft)
        }
        Err(error) => {
            let mut fallback = parse_routing_preference(source, base_policy);
            fallback.warnings.push(format!(
                "Orchestration model interpretation failed; deterministic fallback used: {}",
                display_error(error)
            ));
            Ok(fallback)
        }
    }
}

fn parse_model_routing_draft(json: &str) -> Result<NaturalLanguageRoutingDraft, String> {
    let mut value: serde_json::Value = serde_json::from_str(json).map_err(display_error)?;
    if let Some(preferences) = value
        .pointer_mut("/policy/rolePreferences")
        .and_then(serde_json::Value::as_array_mut)
    {
        for preference in preferences {
            if let Some(object) = preference.as_object_mut() {
                object
                    .entry("hardConstraint")
                    .or_insert(serde_json::Value::Bool(false));
            }
            if let Some(role) = preference.get_mut("role") {
                if let Some(normalized) = role.as_str().and_then(normalize_model_role) {
                    *role = serde_json::Value::String(normalized.to_owned());
                }
            }
            if let Some(providers) = preference
                .get_mut("preferredProviders")
                .and_then(serde_json::Value::as_array_mut)
            {
                for provider in providers {
                    if let Some(normalized) = provider.as_str().and_then(normalize_provider_type) {
                        *provider = serde_json::Value::String(normalized.to_owned());
                    }
                }
            }
        }
    }
    serde_json::from_value(value).map_err(display_error)
}

fn normalize_model_role(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "coding" | "code" | "developer" | "backend" | "software_engineering" => Some("programming"),
        "academic" | "academic_writing" | "paper" | "long_form_writing" => Some("writing"),
        "general" | "worker" => Some("general_worker"),
        "cheap" | "routine" | "fast" => Some("fast_cheap"),
        "game" | "game_dev" => Some("game_development"),
        "orchestration" | "general_worker" | "programming" | "research" | "writing"
        | "frontend" | "game_development" | "reviewer" | "verifier" | "fast_cheap" | "vision" => {
            Some(match value.trim().to_ascii_lowercase().as_str() {
                "orchestration" => "orchestration",
                "general_worker" => "general_worker",
                "programming" => "programming",
                "research" => "research",
                "writing" => "writing",
                "frontend" => "frontend",
                "game_development" => "game_development",
                "reviewer" => "reviewer",
                "verifier" => "verifier",
                "fast_cheap" => "fast_cheap",
                _ => "vision",
            })
        }
        _ => None,
    }
}

fn normalize_provider_type(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai" | "open_ai" | "gpt" => Some("open_ai"),
        "anthropic" | "claude" => Some("anthropic"),
        "deepseek" | "deep_seek" => Some("deep_seek"),
        "compatible" | "openai_compatible" | "open_ai_compatible" => Some("compatible"),
        _ => None,
    }
}

#[tauri::command]
fn get_model_selection_settings(
    state: State<'_, AppState>,
    scope_id: String,
) -> Result<Option<ModelSelectionSettings>, String> {
    state
        .store
        .model_selection_settings(&scope_id)
        .map_err(display_error)
}

#[tauri::command]
fn save_model_selection_settings(
    state: State<'_, AppState>,
    scope_id: String,
    settings: ModelSelectionSettings,
) -> Result<(), String> {
    let providers = state.store.provider_configs().map_err(display_error)?;
    validate_model_selection_settings(&settings, &providers).map_err(display_error)?;
    state
        .store
        .save_model_selection_settings(&scope_id, &settings)
        .map_err(display_error)
}

#[tauri::command]
fn extension_directories() -> Result<ExtensionDirectories, String> {
    ensure_extension_directories()
}

#[tauri::command]
fn discover_skills() -> Result<Vec<SkillSummary>, String> {
    Ok(discover_skill_catalog()?.summaries())
}

#[tauri::command]
fn load_skill(catalog_id: String, allow_once: bool) -> Result<LoadedSkill, String> {
    let directories = ensure_extension_directories()?;
    let catalog = discover_skill_catalog()?;
    let summary = catalog
        .summary(&catalog_id)
        .ok_or_else(|| format!("skill not found after discovery: {catalog_id}"))?;
    let request = PermissionRequest {
        permission: PermissionKind::SkillLoad,
        context: PermissionContext {
            workspace_root: directories.root,
            target_path: Some(summary.path.clone()),
            tool_id: Some("skill.load".into()),
            ..PermissionContext::default()
        },
        risk: RiskLevel::Low,
        action: format!("load skill instructions for {}", summary.name),
    };
    enforce_permissions(&[request], allow_once)?;
    catalog.load(&catalog_id).map_err(display_error)
}

#[tauri::command]
fn list_mcp_server_configs(state: State<'_, AppState>) -> Result<Vec<McpServerConfig>, String> {
    state.store.mcp_server_configs().map_err(display_error)
}

#[tauri::command]
fn save_mcp_server_config(
    state: State<'_, AppState>,
    config: McpServerConfig,
    credential: Option<McpCredentialInput>,
) -> Result<(), String> {
    validate_mcp_config(&config).map_err(display_error)?;
    let credential_rollback = if let Some(credential) = credential {
        if !mcp_credential_references(&config).contains(&credential.reference_id) {
            return Err(format!(
                "credential reference is not used by this MCP configuration: {}",
                credential.reference_id
            ));
        }
        let previous_secret = state
            .mcp_credentials
            .get_optional(&config.id, &credential.reference_id)
            .map_err(display_error)?;
        let secret = McpSecretValue::new(credential.secret).map_err(display_error)?;
        state
            .mcp_credentials
            .put(&config.id, &credential.reference_id, &secret)
            .map_err(display_error)?;
        Some((credential.reference_id, previous_secret))
    } else {
        None
    };

    if let Err(error) = state.store.save_mcp_server_config(&config) {
        if let Some((reference_id, previous_secret)) = credential_rollback {
            let rollback = match previous_secret {
                Some(secret) => state
                    .mcp_credentials
                    .put(&config.id, &reference_id, &secret),
                None => state.mcp_credentials.delete(&config.id, &reference_id),
            };
            return Err(match rollback {
                Ok(()) => format!(
                    "MCP metadata was not saved; the previous credential state was restored: {error}"
                ),
                Err(rollback_error) => format!(
                    "MCP metadata was not saved and credential rollback failed: {error}; {rollback_error}"
                ),
            });
        }
        return Err(display_error(error));
    }
    Ok(())
}

#[tauri::command]
fn delete_mcp_server_config(state: State<'_, AppState>, config_id: String) -> Result<bool, String> {
    state
        .store
        .delete_mcp_server_config(&config_id)
        .map_err(display_error)
}

#[tauri::command]
fn delete_mcp_credential(
    state: State<'_, AppState>,
    server_config_id: String,
    reference_id: String,
) -> Result<(), String> {
    state
        .mcp_credentials
        .delete(&server_config_id, &reference_id)
        .map_err(display_error)
}

#[tauri::command]
async fn test_mcp_server(
    state: State<'_, AppState>,
    workspace_root: String,
    server_config_id: String,
    allow_once: bool,
) -> Result<McpServerStatus, String> {
    let config = stored_mcp_config(&state, &server_config_id)?;
    enforce_permissions(
        &mcp_permission_requests(&workspace_root, &config, None)?,
        allow_once,
    )?;
    NativeMcpClient::new(&state.mcp_credentials)
        .health_check(&config, CancellationToken::new())
        .await
        .map_err(display_error)
}

#[tauri::command]
async fn invoke_mcp_tool(
    state: State<'_, AppState>,
    workspace_root: String,
    server_config_id: String,
    tool_name: String,
    arguments: serde_json::Value,
    allow_once: bool,
) -> Result<McpInvocationResult, String> {
    let config = stored_mcp_config(&state, &server_config_id)?;
    enforce_permissions(
        &mcp_permission_requests(&workspace_root, &config, Some(&tool_name))?,
        allow_once,
    )?;
    NativeMcpClient::new(&state.mcp_credentials)
        .invoke(&config, &tool_name, arguments, CancellationToken::new())
        .await
        .map_err(display_error)
}

#[tauri::command]
async fn preview_github_import(
    workspace_root: String,
    url: String,
    allow_once: bool,
) -> Result<GithubImportPreview, String> {
    let workspace = canonical_workspace(&workspace_root)?;
    enforce_permissions(
        &github_preview_permission_requests(&workspace, &url),
        allow_once,
    )?;
    tauri::async_runtime::spawn_blocking(move || {
        GithubImportManager::new(data_root()?)
            .and_then(|manager| manager.preview(&url))
            .map_err(display_error)
    })
    .await
    .map_err(display_error)?
}

pub(crate) async fn install_user_skills_from_github(
    url: String,
) -> Result<serde_json::Value, String> {
    let directories = ensure_extension_directories()?;
    let manager_root = data_root()?;
    let preview = tauri::async_runtime::spawn_blocking({
        let manager_root = manager_root.clone();
        move || {
            GithubImportManager::new(manager_root)
                .and_then(|manager| manager.preview(&url))
                .map_err(display_error)
        }
    })
    .await
    .map_err(display_error)??;
    if preview.blocked {
        return Err("the GitHub import was blocked by the inert-content security scan".to_owned());
    }
    let component_ids = preview
        .components
        .iter()
        .filter(|component| component.kind == lunascope_core::ImportComponentKind::Skill)
        .map(|component| component.id.clone())
        .collect::<Vec<_>>();
    if component_ids.is_empty() {
        return Err("no compatible SKILL.md component was found in the repository".to_owned());
    }
    let request = ImportInstallRequest {
        import_id: preview.import_id.clone(),
        workspace_root: directories.user_skills,
        component_ids,
    };
    let installed = tauri::async_runtime::spawn_blocking(move || {
        GithubImportManager::new(manager_root)
            .and_then(|manager| manager.install(&request))
            .map_err(display_error)
    })
    .await
    .map_err(display_error)??;
    Ok(serde_json::json!({
        "installed": installed,
        "commitSha": preview.commit_sha,
        "license": preview.license,
        "warnings": preview.warnings,
        "contentExecuted": false
    }))
}

#[tauri::command]
async fn install_github_import(
    request: ImportInstallRequest,
    allow_once: bool,
) -> Result<InstalledImportVersion, String> {
    let workspace = canonical_workspace(&request.workspace_root)?;
    let context = PermissionContext {
        workspace_root: workspace.to_string_lossy().into_owned(),
        tool_id: Some("github.import.install".into()),
        ..PermissionContext::default()
    };
    enforce_permissions(
        &[
            PermissionRequest {
                permission: PermissionKind::FilesystemWrite,
                context: PermissionContext {
                    target_path: Some(
                        workspace
                            .join(".lunascope")
                            .join("imports")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    ..context.clone()
                },
                risk: RiskLevel::Medium,
                action: "install explicitly selected pinned import components".into(),
            },
            PermissionRequest {
                permission: PermissionKind::ProcessSpawn,
                context: PermissionContext {
                    program: Some("git.exe".into()),
                    arguments: vec![
                        "cat-file".into(),
                        "blob".into(),
                        "<pinned-object-id>".into(),
                    ],
                    ..context
                },
                risk: RiskLevel::Medium,
                action: "read pinned Git blobs without checkout or content execution".into(),
            },
        ],
        allow_once,
    )?;
    tauri::async_runtime::spawn_blocking(move || {
        GithubImportManager::new(data_root()?)
            .and_then(|manager| manager.install(&request))
            .map_err(display_error)
    })
    .await
    .map_err(display_error)?
}

#[tauri::command]
fn list_installed_imports() -> Result<Vec<InstalledImportVersion>, String> {
    GithubImportManager::new(data_root()?)
        .and_then(|manager| manager.list_installed())
        .map_err(display_error)
}

#[tauri::command]
async fn rollback_github_import(
    workspace_root: String,
    installation_id: String,
    version_id: String,
    allow_once: bool,
) -> Result<InstalledImportVersion, String> {
    let workspace = canonical_workspace(&workspace_root)?;
    enforce_permissions(
        &[PermissionRequest {
            permission: PermissionKind::FilesystemWrite,
            context: PermissionContext {
                workspace_root: workspace.to_string_lossy().into_owned(),
                target_path: Some(
                    workspace
                        .join(".lunascope")
                        .join("imports")
                        .to_string_lossy()
                        .into_owned(),
                ),
                tool_id: Some("github.import.rollback".into()),
                ..PermissionContext::default()
            },
            risk: RiskLevel::Medium,
            action: format!("restore pinned import version {version_id}"),
        }],
        allow_once,
    )?;
    tauri::async_runtime::spawn_blocking(move || {
        GithubImportManager::new(data_root()?)
            .and_then(|manager| manager.rollback(&installation_id, &version_id))
            .map_err(display_error)
    })
    .await
    .map_err(display_error)?
}

#[tauri::command]
fn compare_github_import(
    installation_id: String,
    candidate_import_id: String,
) -> Result<ImportComparison, String> {
    GithubImportManager::new(data_root()?)
        .and_then(|manager| manager.compare(&installation_id, &candidate_import_id))
        .map_err(display_error)
}

#[tauri::command]
fn route_tools(
    workspace_root: String,
    mut request: ToolRoutingRequest,
) -> Result<ToolRoutingDecision, String> {
    let workspace = canonical_workspace(&workspace_root)?;
    let catalog = discover_skill_catalog()?;
    request.permission_context.workspace_root = workspace.to_string_lossy().into_owned();
    Ok(ToolRouter.route(
        &request,
        &native_tool_manifests(),
        &catalog.summaries(),
        &PolicyEngine::default(),
    ))
}

fn discover_skill_catalog() -> Result<SkillCatalog, String> {
    let directories = ensure_extension_directories()?;
    let roots = vec![
        SkillRoot {
            source: SkillSourceKind::LunaScopeSystem,
            path: PathBuf::from(directories.system_skills),
        },
        SkillRoot {
            source: SkillSourceKind::LunaScopeUser,
            path: PathBuf::from(directories.user_skills),
        },
    ];
    SkillCatalog::discover(&roots).map_err(display_error)
}

fn ensure_extension_directories() -> Result<ExtensionDirectories, String> {
    let _guard = EXTENSION_DIRECTORY_LOCK
        .lock()
        .map_err(|_| "extension directory lock is poisoned".to_owned())?;
    let root = data_root()?.join("extensions");
    let system_skills = root.join("skills").join("system");
    let user_skills = root.join("skills").join("user");
    let tools = root.join("tools");
    let mcp = root.join("mcp");
    for path in [&system_skills, &user_skills, &tools, &mcp] {
        fs::create_dir_all(path).map_err(display_error)?;
    }
    seed_builtin_system_skills(&system_skills)?;
    Ok(ExtensionDirectories {
        root: root.to_string_lossy().into_owned(),
        system_skills: system_skills.to_string_lossy().into_owned(),
        user_skills: user_skills.to_string_lossy().into_owned(),
        tools: tools.to_string_lossy().into_owned(),
        mcp: mcp.to_string_lossy().into_owned(),
    })
}

fn seed_builtin_system_skills(destination: &Path) -> Result<(), String> {
    for file in BUILTIN_SYSTEM_SKILLS.files() {
        let path = destination.join(file.path());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(display_error)?;
        }
        let bytes = file.contents();
        let unchanged = fs::read(&path).is_ok_and(|current| current == bytes);
        if !unchanged {
            fs::write(&path, bytes).map_err(display_error)?;
        }
    }
    for directory in BUILTIN_SYSTEM_SKILLS.dirs() {
        seed_builtin_skill_directory(destination, directory)?;
    }
    let mut retained_paths = BTreeSet::new();
    collect_builtin_skill_paths(&BUILTIN_SYSTEM_SKILLS, &mut retained_paths);
    prune_stale_system_skill_files(destination, destination, &retained_paths)?;
    Ok(())
}

fn seed_builtin_skill_directory(destination: &Path, directory: &Dir<'_>) -> Result<(), String> {
    for file in directory.files() {
        let path = destination.join(file.path());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(display_error)?;
        }
        let bytes = file.contents();
        let unchanged = fs::read(&path).is_ok_and(|current| current == bytes);
        if !unchanged {
            fs::write(&path, bytes).map_err(display_error)?;
        }
    }
    for child in directory.dirs() {
        seed_builtin_skill_directory(destination, child)?;
    }
    Ok(())
}

fn collect_builtin_skill_paths(directory: &Dir<'_>, paths: &mut BTreeSet<PathBuf>) {
    paths.extend(directory.files().map(|file| file.path().to_path_buf()));
    for child in directory.dirs() {
        collect_builtin_skill_paths(child, paths);
    }
}

fn prune_stale_system_skill_files(
    root: &Path,
    directory: &Path,
    retained_paths: &BTreeSet<PathBuf>,
) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(display_error(error)),
    };
    for entry in entries {
        let entry = entry.map_err(display_error)?;
        let file_type = entry.file_type().map_err(display_error)?;
        let path = entry.path();
        if file_type.is_dir() {
            prune_stale_system_skill_files(root, &path, retained_paths)?;
            let is_empty = match fs::read_dir(&path) {
                Ok(mut entries) => entries.next().is_none(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(display_error(error)),
            };
            if is_empty {
                if let Err(error) = fs::remove_dir(&path) {
                    if !matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) {
                        return Err(display_error(error));
                    }
                }
            }
            continue;
        }
        let relative = path.strip_prefix(root).map_err(display_error)?;
        if !retained_paths.contains(relative) {
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(display_error(error));
                }
            }
        }
    }
    Ok(())
}

fn ensure_builtin_mcp_configs(store: &SqliteEventStore) -> Result<(), String> {
    let existing = store.mcp_server_configs().map_err(display_error)?;
    if existing
        .iter()
        .any(|config| config.id == "builtin-openai-developer-docs")
    {
        return Ok(());
    }
    store
        .save_mcp_server_config(&McpServerConfig {
            id: "builtin-openai-developer-docs".into(),
            name: "OpenAI Developer Docs (built-in)".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "https://developers.openai.com/mcp".into(),
                headers: Vec::new(),
            },
            timeout_ms: 30_000,
            enabled: true,
        })
        .map_err(display_error)
}

pub(crate) fn canonical_workspace(workspace_root: &str) -> Result<PathBuf, String> {
    let workspace = PathBuf::from(workspace_root);
    if !workspace.is_absolute() {
        return Err("workspace root must be an absolute path".into());
    }
    let workspace = workspace.canonicalize().map_err(display_error)?;
    if !workspace.is_dir() {
        return Err("workspace root must be a directory".into());
    }
    Ok(workspace)
}

fn github_preview_permission_requests(workspace: &Path, url: &str) -> Vec<PermissionRequest> {
    let context = PermissionContext {
        workspace_root: workspace.to_string_lossy().into_owned(),
        tool_id: Some("github.import.preview".into()),
        ..PermissionContext::default()
    };
    vec![
        PermissionRequest {
            permission: PermissionKind::NetworkConnect,
            context: PermissionContext {
                network_domain: Some("github.com".into()),
                ..context.clone()
            },
            risk: RiskLevel::Medium,
            action: format!("download GitHub objects into quarantine from {url}"),
        },
        PermissionRequest {
            permission: PermissionKind::ProcessSpawn,
            context: PermissionContext {
                program: Some("git.exe".into()),
                arguments: vec!["fetch".into(), "--filter=blob:limit=16777216".into()],
                ..context
            },
            risk: RiskLevel::Medium,
            action: "use trusted Git for object transfer only; no checkout or hooks".into(),
        },
    ]
}

fn stored_mcp_config(
    state: &State<'_, AppState>,
    server_config_id: &str,
) -> Result<McpServerConfig, String> {
    state
        .store
        .mcp_server_configs()
        .map_err(display_error)?
        .into_iter()
        .find(|config| config.id == server_config_id)
        .ok_or_else(|| format!("MCP server configuration not found: {server_config_id}"))
}

fn mcp_permission_requests(
    workspace_root: &str,
    config: &McpServerConfig,
    tool_name: Option<&str>,
) -> Result<Vec<PermissionRequest>, String> {
    let workspace = PathBuf::from(workspace_root);
    if !workspace.is_absolute() {
        return Err("workspace root must be an absolute path".into());
    }
    let workspace = workspace.canonicalize().map_err(display_error)?;
    let tool_id = tool_name
        .map(|name| format!("mcp.{}.{}", config.id, name))
        .unwrap_or_else(|| format!("mcp.{}.health", config.id));
    let action = tool_name
        .map(|name| format!("invoke MCP tool {} on {}", name, config.name))
        .unwrap_or_else(|| format!("check MCP server {}", config.name));
    let base = PermissionContext {
        workspace_root: workspace.to_string_lossy().into_owned(),
        tool_id: Some(tool_id),
        ..PermissionContext::default()
    };
    let mut requests = vec![PermissionRequest {
        permission: PermissionKind::McpInvoke,
        context: base.clone(),
        risk: RiskLevel::Medium,
        action: action.clone(),
    }];
    match &config.transport {
        McpTransportConfig::Stdio { program, args, .. } => {
            let mut context = base.clone();
            context.program = Some(program.clone());
            context.arguments = args.clone();
            requests.push(PermissionRequest {
                permission: PermissionKind::ProcessSpawn,
                context,
                risk: RiskLevel::High,
                action: format!("spawn MCP server process {}", config.name),
            });
        }
        McpTransportConfig::StreamableHttp { url, .. } => {
            let parsed = url::Url::parse(url).map_err(display_error)?;
            let mut context = base.clone();
            context.network_domain = parsed.host_str().map(str::to_owned);
            requests.push(PermissionRequest {
                permission: PermissionKind::NetworkConnect,
                context,
                risk: RiskLevel::Medium,
                action: format!("connect to MCP server {}", config.name),
            });
        }
    }
    if !mcp_credential_references(config).is_empty() {
        requests.push(PermissionRequest {
            permission: PermissionKind::SecretsUse,
            context: base,
            risk: RiskLevel::High,
            action: format!("use credential references for MCP server {}", config.name),
        });
    }
    Ok(requests)
}

fn mcp_credential_references(config: &McpServerConfig) -> Vec<String> {
    let values: Vec<&McpConfigValue> = match &config.transport {
        McpTransportConfig::Stdio { environment, .. } => environment.values().collect(),
        McpTransportConfig::StreamableHttp { headers, .. } => {
            headers.iter().map(|header| &header.value).collect()
        }
    };
    values
        .into_iter()
        .filter_map(|value| match value {
            McpConfigValue::CredentialReference(reference) => Some(reference.clone()),
            McpConfigValue::Literal(_) => None,
        })
        .collect()
}

pub(crate) fn enforce_permissions(
    requests: &[PermissionRequest],
    allow_once: bool,
) -> Result<(), String> {
    let policy = PolicyEngine::default();
    let mut approval_reasons = Vec::new();
    for request in requests {
        let outcome = policy.evaluate(request);
        match outcome.decision {
            PolicyDecision::Deny => {
                return Err(format!("permission denied: {}", outcome.reason));
            }
            PolicyDecision::Ask if !allow_once => approval_reasons.push(format!(
                "{:?}: {} ({})",
                request.permission, request.action, outcome.reason
            )),
            PolicyDecision::Allow | PolicyDecision::Ask => {}
        }
    }
    if approval_reasons.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "APPROVAL_REQUIRED: {}",
            approval_reasons.join("; ")
        ))
    }
}

fn native_tool_manifests() -> Vec<ToolManifest> {
    vec![
        tool_manifest(
            "filesystem.read",
            "Read workspace files",
            RiskLevel::Low,
            vec![PermissionKind::FilesystemRead],
            true,
        ),
        tool_manifest(
            "filesystem.apply_text_patch",
            "Apply a bounded text patch",
            RiskLevel::Medium,
            vec![PermissionKind::FilesystemWrite],
            false,
        ),
        tool_manifest(
            "process.run",
            "Run an approved child process",
            RiskLevel::High,
            vec![PermissionKind::ProcessSpawn],
            false,
        ),
        tool_manifest(
            "git.diff",
            "Inspect repository changes",
            RiskLevel::Low,
            vec![PermissionKind::FilesystemRead],
            true,
        ),
        tool_manifest(
            "mcp.search",
            "Use a discovered MCP search tool",
            RiskLevel::Medium,
            vec![PermissionKind::McpInvoke],
            false,
        ),
        tool_manifest(
            "browser.navigate",
            "Navigate an approved browser session",
            RiskLevel::Medium,
            vec![PermissionKind::BrowserControl],
            false,
        ),
        tool_manifest(
            "skill.load",
            "Load skill instructions on demand",
            RiskLevel::Low,
            vec![PermissionKind::SkillLoad],
            true,
        ),
    ]
}

fn tool_manifest(
    id: &str,
    description: &str,
    risk: RiskLevel,
    permissions: Vec<PermissionKind>,
    deterministic: bool,
) -> ToolManifest {
    ToolManifest {
        id: id.into(),
        name: id.into(),
        description: description.into(),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        risk,
        permissions,
        timeout_ms: 30_000,
        cancellable: true,
        deterministic,
        source: "lunascope-native".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

pub(crate) fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(crate) fn data_root() -> Result<PathBuf, String> {
    let drive = Path::new(r"D:\");
    if !drive.exists() {
        return Err(
            "D: drive is unavailable; first-run data-directory selection is required".into(),
        );
    }
    Ok(drive.join("LunaScopeData"))
}

#[tauri::command]
fn default_workspace_root() -> Result<String, String> {
    std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(display_error)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let root = data_root().map_err(std::io::Error::other)?;
            let state_dir = root.join("state");
            fs::create_dir_all(&state_dir)?;
            let store = SqliteEventStore::open(state_dir.join("lunascope.db"))
                .map_err(std::io::Error::other)?;
            ensure_extension_directories().map_err(std::io::Error::other)?;
            ensure_builtin_mcp_configs(&store).map_err(std::io::Error::other)?;
            app.manage(AppState {
                store: Arc::new(store),
                credentials: KeyringCredentialStore,
                mcp_credentials: McpCredentialStore,
                active_orchestration: Mutex::new(None),
            });
            companion::restore_window_visibility(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_run,
            delete_conversation,
            get_runtime_snapshot,
            subscribe_run,
            list_provider_configs,
            save_provider_config,
            delete_provider_credential,
            test_provider_connection,
            get_routing_policy,
            save_routing_policy,
            parse_model_preference,
            get_model_selection_settings,
            save_model_selection_settings,
            extension_directories,
            discover_skills,
            load_skill,
            list_mcp_server_configs,
            save_mcp_server_config,
            delete_mcp_server_config,
            delete_mcp_credential,
            test_mcp_server,
            invoke_mcp_tool,
            preview_github_import,
            install_github_import,
            list_installed_imports,
            rollback_github_import,
            compare_github_import,
            attachment::import_attachments,
            attachment::remove_imported_attachment,
            companion::companion_get_settings,
            companion::companion_save_preferences,
            companion::companion_import_model,
            companion::companion_set_activity,
            companion::companion_avatar_requirements,
            companion::companion_list_avatar_packs,
            companion::companion_create_avatar_pack,
            companion::companion_duplicate_avatar_pack,
            companion::companion_delete_avatar_pack,
            companion::companion_repack_avatar_pack,
            companion::companion_load_avatar_manifest,
            companion::companion_save_avatar_manifest,
            companion::companion_validate_avatar_pack,
            companion::companion_register_avatar_pack,
            companion::companion_install_avatar_pack,
            companion::companion_import_avatar_layers,
            companion::companion_read_avatar_asset,
            companion::models::companion_search_catalog,
            companion::models::companion_list_installed_models,
            companion::models::companion_install_catalog_model,
            companion::models::companion_activate_installed_model,
            companion::models::companion_remove_installed_model,
            orchestration::draft_native_orchestration,
            orchestration::list_conversation_messages,
            orchestration::list_domain_packs,
            orchestration::patch_native_orchestration,
            orchestration::run_native_orchestration,
            orchestration::read_orchestration_change_set,
            orchestration::recover_native_orchestration_view,
            orchestration::retry_failed_native_orchestration,
            orchestration::guide_native_orchestration,
            orchestration::set_native_orchestration_paused,
            orchestration::cancel_native_orchestration,
            ultranote::list_courses,
            ultranote::create_course,
            ultranote::bind_course_thread,
            ultranote::activate_ultranote,
            ultranote::search_ultranote,
            ultranote::ingest_syllabus,
            ultranote::ingest_lecture,
            ultranote::evaluate_homework,
            ultranote::export_ultranote,
            project::list_projects,
            project::save_project,
            project::delete_project,
            project::get_user_preferences,
            project::save_user_preferences,
            project::read_project_file,
            project::create_project_file,
            project::patch_project_file,
            route_tools,
            default_workspace_root
        ])
        .run(tauri::generate_context!())
        .expect("failed to run LunaScope desktop");
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunascope_core::McpHeader;

    fn repository_root() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repository root")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn system_and_user_skills_use_separate_global_directories() {
        let directories = ensure_extension_directories().expect("extension directories");
        assert_ne!(directories.system_skills, directories.user_skills);
        assert!(
            directories
                .system_skills
                .ends_with(r"extensions\skills\system")
        );
        assert!(directories.user_skills.ends_with(r"extensions\skills\user"));
        assert!(Path::new(&directories.tools).is_dir());
        assert!(Path::new(&directories.mcp).is_dir());
        let catalog = discover_skill_catalog().expect("bundled skill catalog");
        let names = catalog
            .summaries()
            .into_iter()
            .filter(|skill| skill.source == SkillSourceKind::LunaScopeSystem)
            .map(|skill| skill.name)
            .collect::<Vec<_>>();
        for required in [
            "systematic-debugging",
            "frontend-design",
            "autoresearch",
            "ml-paper-writing",
            "playwright",
            "verification-before-completion",
            "deep-research",
            "academic-pipeline",
            "mermaid-skill",
        ] {
            assert!(
                names.iter().any(|name| name == required),
                "missing {required}"
            );
        }
        let pipeline = catalog
            .summaries()
            .into_iter()
            .find(|skill| skill.name == "academic-pipeline")
            .expect("academic pipeline Skill");
        let shared_reference = catalog
            .read_resource(
                &pipeline.catalog_id,
                "shared/references/intent_clarification_protocol.md",
            )
            .expect("Imbad0202 shared Skill resource");
        assert!(shared_reference.contains("Intent Clarification"));
        let claude_instructions = catalog
            .read_resource(&pipeline.catalog_id, ".claude/CLAUDE.md")
            .expect("Claude package instructions");
        assert!(claude_instructions.contains("Routing"));
    }

    #[test]
    fn bundled_system_skill_seed_reconciles_stale_files() {
        let temporary = tempfile::tempdir().expect("temporary system Skill root");
        let stale = temporary.path().join("obsolete").join("STALE.md");
        fs::create_dir_all(stale.parent().expect("stale parent")).expect("stale directory");
        fs::write(&stale, "obsolete").expect("stale file");

        seed_builtin_system_skills(temporary.path()).expect("seed bundled Skills");

        assert!(!stale.exists());
        assert!(temporary.path().join("research").join("SKILL.md").is_file());
        assert!(temporary.path().join("UPSTREAM_SOURCES.md").is_file());
    }

    #[test]
    fn built_in_mcp_seed_is_real_and_idempotent() {
        let temporary = tempfile::tempdir().expect("temporary store");
        let store = SqliteEventStore::open(temporary.path().join("state.db")).expect("event store");
        ensure_builtin_mcp_configs(&store).expect("first seed");
        ensure_builtin_mcp_configs(&store).expect("second seed");
        let configs = store.mcp_server_configs().expect("mcp configs");
        let builtins = configs
            .iter()
            .filter(|config| config.id == "builtin-openai-developer-docs")
            .collect::<Vec<_>>();
        assert_eq!(builtins.len(), 1);
        assert!(builtins[0].enabled);
        assert!(matches!(
            &builtins[0].transport,
            McpTransportConfig::StreamableHttp { url, headers }
                if url == "https://developers.openai.com/mcp" && headers.is_empty()
        ));
    }

    #[test]
    fn routing_draft_normalizes_common_model_aliases_without_changing_policy_shape() {
        let draft = parse_model_routing_draft(
            r#"{
              "sourceText":"coding uses DeepSeek",
              "policy":{
                "priorities":{"quality":80,"cost":50,"speed":50,"privacy":50},
                "allowedProviderConfigIds":[],"disallowedProviderConfigIds":[],
                "maximumCostMicrousdPerRun":null,"preferLocal":false,
                "requiredCapabilities":["text"],"fallbackAllowed":true,
                "askBeforeCostEscalation":true,
                "rolePreferences":[{"role":"coding","preferredProviders":["deepseek"]}]
              },
              "matchedRules":["coding prefers DeepSeek"],"warnings":[],"confirmed":false
            }"#,
        )
        .expect("normalized routing draft");
        assert_eq!(
            draft.policy.role_preferences[0].role,
            lunascope_core::ModelRole::Programming
        );
        assert_eq!(
            draft.policy.role_preferences[0].preferred_providers,
            vec![ProviderType::DeepSeek]
        );
    }

    #[test]
    #[ignore = "requires the user's configured DeepSeek credential and live network access"]
    fn deepseek_v4_flash_interprets_natural_language_routing() {
        let store = Arc::new(SqliteEventStore::open_in_memory().expect("event store"));
        store
            .save_provider_config(&ProviderConfig {
                id: "deepseek-primary".to_owned(),
                provider_type: ProviderType::DeepSeek,
                protocol: lunascope_core::ProviderProtocol::OpenAiChatCompletions,
                display_name: "DeepSeek official".to_owned(),
                base_url: "https://api.deepseek.com".to_owned(),
                credential_reference_id: "deepseek-primary".to_owned(),
                default_model_id: "deepseek-v4-flash".to_owned(),
                custom_headers: Vec::new(),
                context_window_tokens: None,
                supports_tools: true,
                supports_vision: false,
                supports_structured_output: true,
                enabled: true,
            })
            .expect("provider config");
        let state = AppState {
            store,
            credentials: KeyringCredentialStore,
            mcp_credentials: McpCredentialStore,
            active_orchestration: Mutex::new(None),
        };
        let policy = ModelRoutingPolicy {
            priorities: lunascope_core::RoutingPriorities {
                quality: 80,
                cost: 50,
                speed: 50,
                privacy: 50,
            },
            allowed_provider_config_ids: Vec::new(),
            disallowed_provider_config_ids: Vec::new(),
            maximum_cost_microusd_per_run: None,
            prefer_local: false,
            required_capabilities: vec![lunascope_core::ModelCapability::Text],
            fallback_allowed: true,
            ask_before_cost_escalation: true,
            role_preferences: Vec::new(),
        };
        let draft = tauri::async_runtime::block_on(parse_model_preference_with_state(
            &state,
            "编程任务优先 DeepSeek，论文写作优先 OpenAI；模型不可用时允许回退。".to_owned(),
            policy,
            true,
        ))
        .expect("LLM routing draft");
        assert!(!draft.matched_rules.is_empty());
        assert!(draft.policy.role_preferences.iter().any(|preference| {
            preference.role == lunascope_core::ModelRole::Programming
                && preference
                    .preferred_providers
                    .contains(&ProviderType::DeepSeek)
        }));
        assert!(
            !draft
                .warnings
                .iter()
                .any(|warning| warning.contains("deterministic fallback"))
        );
    }

    #[test]
    fn mcp_health_requires_explicit_approval_for_every_sensitive_capability() {
        let root = repository_root();
        let config = McpServerConfig {
            id: "remote-docs".into(),
            name: "Remote docs".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".into(),
                headers: vec![McpHeader {
                    name: "Authorization".into(),
                    value: McpConfigValue::CredentialReference("bearer".into()),
                }],
            },
            timeout_ms: 30_000,
            enabled: true,
        };
        let requests = mcp_permission_requests(&root, &config, None).expect("requests");
        assert_eq!(
            requests
                .iter()
                .map(|request| request.permission)
                .collect::<Vec<_>>(),
            [
                PermissionKind::McpInvoke,
                PermissionKind::NetworkConnect,
                PermissionKind::SecretsUse
            ]
        );
        assert!(
            enforce_permissions(&requests, false)
                .expect_err("approval required")
                .starts_with("APPROVAL_REQUIRED:")
        );
        enforce_permissions(&requests, true).expect("allow once");
    }
}
