use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticCode {
    DataRootUnavailable,
    DataRootNotWritable,
    GitNotFound,
    ProviderNotConfigured,
    ProviderAuthFailed,
    ProviderModelUnsupported,
    ModelReasoningTestRequired,
    WorkspaceInvalid,
    WorkspacePermissionDenied,
    BrowserNotAvailable,
    ToolDependencyMissing,
    RunInterrupted,
    RecoveryNeedsIntervention,
    UnexpectedFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ActionableDiagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub title: String,
    pub what_happened: String,
    pub why: String,
    pub how_to_fix: Vec<String>,
    pub technical_detail: Option<String>,
    pub retryable: bool,
}

impl ActionableDiagnostic {
    pub fn error(
        code: DiagnosticCode,
        title: impl Into<String>,
        what_happened: impl Into<String>,
        why: impl Into<String>,
        how_to_fix: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Error,
            title: title.into(),
            what_happened: what_happened.into(),
            why: why.into(),
            how_to_fix: how_to_fix.into_iter().map(Into::into).collect(),
            technical_detail: None,
            retryable: true,
        }
    }

    pub fn with_technical_detail(mut self, detail: impl Into<String>) -> Self {
        self.technical_detail = Some(detail.into());
        self
    }
}

pub fn classify_error(message: &str) -> ActionableDiagnostic {
    let redacted = redact_sensitive_text(message);
    let normalized = redacted.to_ascii_lowercase();
    let (code, title, what_happened, why, fixes) = if normalized.contains("data_root_not_writable")
    {
        (
            DiagnosticCode::DataRootNotWritable,
            "LunaScope cannot write its data directory",
            "The configured data directory is not writable.",
            "The database, extensions, worktrees, and local model assets must be stored in a writable location.",
            vec![
                "Check the directory permissions and free space.",
                "Choose a writable LunaScope data directory and retry.",
            ],
        )
    } else if normalized.contains("data_root_unavailable") {
        (
            DiagnosticCode::DataRootUnavailable,
            "The LunaScope data directory is unavailable",
            "The saved data directory could not be opened.",
            "The drive may be disconnected, the directory may have moved, or its saved configuration may be invalid.",
            vec![
                "Reconnect the drive or restore access to the saved directory.",
                "If the location changed, select the new data directory and retry.",
            ],
        )
    } else if normalized.contains("gitnotfound")
        || normalized.contains("git was not found")
        || normalized.contains("git_not_found")
    {
        (
            DiagnosticCode::GitNotFound,
            "Git is required before Agents can run",
            "LunaScope did not find Git on this computer.",
            "Git worktrees isolate Agent changes and prevent unsafe direct edits.",
            vec![
                "Install Git for Windows from https://git-scm.com/download/win.",
                "Restart LunaScope and retry the environment check.",
            ],
        )
    } else if normalized.contains("provider_not_configured")
        || normalized.contains("provider configuration not found")
        || normalized.contains("enabled provider configuration not found")
    {
        (
            DiagnosticCode::ProviderNotConfigured,
            "Connect an AI provider",
            "The selected Provider configuration is missing or disabled.",
            "LunaScope needs one enabled and tested Provider before it can plan a task.",
            vec!["Open Settings → Models & Providers, save an enabled Provider, and retry."],
        )
    } else if normalized.contains("provider_auth_failed")
        || normalized.contains("401")
        || normalized.contains("403")
        || normalized.contains("unauthorized")
        || normalized.contains("authentication")
    {
        (
            DiagnosticCode::ProviderAuthFailed,
            "The Provider rejected the credential",
            "The Provider authentication check failed.",
            "The API key may be invalid, expired, revoked, or not permitted to use the selected endpoint.",
            vec![
                "Re-enter the credential in Settings → Models & Providers.",
                "Confirm the account can use the selected endpoint and model, then retry.",
            ],
        )
    } else if normalized.contains("model_reasoning_test_required") {
        (
            DiagnosticCode::ModelReasoningTestRequired,
            "Verify the model setting",
            "The model and reasoning setting have not passed the required compatibility test.",
            "Testing before execution prevents a task from failing after Agents have started.",
            vec![
                "Open Settings → Models & Providers and run the model test for the exact setting before retrying.",
            ],
        )
    } else if normalized.contains("provider_model_unsupported")
        || (normalized.contains("model")
            && (normalized.contains("unsupported") || normalized.contains("not found")))
    {
        (
            DiagnosticCode::ProviderModelUnsupported,
            "The selected model is unavailable",
            "The Provider did not accept the selected model or reasoning setting.",
            "Model names and supported reasoning values vary by Provider and endpoint.",
            vec![
                "Check the exact model name in the Provider account.",
                "Use Auto reasoning or a supported value, then retry.",
            ],
        )
    } else if normalized.contains("workspace_permission_denied")
        || (normalized.contains("workspace") && normalized.contains("permission"))
    {
        (
            DiagnosticCode::WorkspacePermissionDenied,
            "Windows blocked the project folder",
            "LunaScope could not access the selected workspace.",
            "Folder permissions or Windows Controlled Folder Access denied access.",
            vec![
                "Choose a folder your Windows account can read and write.",
                "Allow LunaScope in Windows Security if Controlled Folder Access is enabled.",
            ],
        )
    } else if normalized.contains("workspace_invalid") || normalized.contains("workspace") {
        (
            DiagnosticCode::WorkspaceInvalid,
            "Choose a valid project folder",
            "The selected workspace does not exist or is not a directory.",
            "Agents need a real project folder for isolation and verification.",
            vec!["Choose Folder and select the project directory, then retry."],
        )
    } else if normalized.contains("browser_not_available")
        || (normalized.contains("browser")
            && (normalized.contains("unavailable") || normalized.contains("not found")))
    {
        (
            DiagnosticCode::BrowserNotAvailable,
            "A browser required by this task is unavailable",
            "LunaScope did not find Microsoft Edge or Google Chrome.",
            "Browser acceptance is required only for tasks whose verification needs a real page.",
            vec!["Install or repair Microsoft Edge, then retry the task check."],
        )
    } else if normalized.contains("tool_dependency_missing")
        || normalized.contains("program is not available")
    {
        (
            DiagnosticCode::ToolDependencyMissing,
            "A task dependency is missing",
            "A tool required by the selected Worker or verifier was not found.",
            "This dependency is task-specific and is not required for unrelated LunaScope tasks.",
            vec!["Install the named tool, restart LunaScope, and retry."],
        )
    } else if normalized.contains("recovery_needs_intervention") {
        (
            DiagnosticCode::RecoveryNeedsIntervention,
            "This interrupted change needs review",
            "LunaScope cannot prove whether an interrupted mutating tool completed.",
            "Automatically replaying the tool could duplicate or overwrite a real side effect.",
            vec![
                "Inspect the preserved workspace and diagnostics.",
                "Start an explicit repair task after confirming the current file state.",
            ],
        )
    } else if normalized.contains("run_interrupted")
        || normalized.contains("interrupted")
        || normalized.contains("restart")
    {
        (
            DiagnosticCode::RunInterrupted,
            "The run was interrupted",
            "The desktop stopped before the run reached a resumable checkpoint.",
            "Completed artifacts were preserved, but unfinished work must be checked before continuing.",
            vec![
                "Review the preserved Changes and diagnostics, then retry only the unfinished work.",
            ],
        )
    } else {
        (
            DiagnosticCode::UnexpectedFailure,
            "LunaScope could not complete the operation",
            "An unexpected error stopped the current operation.",
            "The technical detail is preserved for diagnosis without changing the security boundary.",
            vec![
                "Copy the redacted diagnostics and retry once.",
                "If it repeats, include the diagnostics in a bug report.",
            ],
        )
    };
    ActionableDiagnostic::error(code, title, what_happened, why, fixes)
        .with_technical_detail(redacted)
}

pub fn redact_sensitive_text(input: &str) -> String {
    let mut value = input.to_owned();
    redact_bearer_tokens(&mut value);
    for marker in [
        "Authorization",
        "api_key",
        "api-key",
        "apiKey",
        "x-api-key",
        "client_secret",
        "access_token",
        "refresh_token",
        "credentialValue",
        "credential_value",
        "credential",
        "Set-Cookie",
        "Cookie",
        "Token",
        "Password",
        "Secret",
    ] {
        let mut search_from = 0;
        while let Some(marker_start) = find_ascii_case_insensitive(&value, marker, search_from) {
            let marker_end = marker_start + marker.len();
            if !is_word_boundary(&value, marker_start, marker_end) {
                search_from = marker_end;
                continue;
            }
            let mut separator = marker_end;
            while value[separator..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            {
                separator += value[separator..].chars().next().unwrap().len_utf8();
            }
            if !matches!(value[separator..].chars().next(), Some(':' | '=')) {
                search_from = marker_end;
                continue;
            }
            separator += 1;
            while value[separator..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            {
                separator += value[separator..].chars().next().unwrap().len_utf8();
            }
            let (start, end) = redacted_value_range(&value, separator);
            if start == end {
                search_from = separator;
                continue;
            }
            value.replace_range(start..end, "[REDACTED]");
            search_from = start + "[REDACTED]".len();
        }
    }
    value
}

fn find_ascii_case_insensitive(input: &str, needle: &str, search_from: usize) -> Option<usize> {
    input[search_from..]
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
        .map(|offset| search_from + offset)
}

fn is_word_boundary(input: &str, start: usize, end: usize) -> bool {
    let is_word =
        |character: char| character.is_ascii_alphanumeric() || matches!(character, '_' | '-');
    input[..start]
        .chars()
        .next_back()
        .is_none_or(|character| !is_word(character))
        && input[end..]
            .chars()
            .next()
            .is_none_or(|character| !is_word(character))
}

fn redact_bearer_tokens(value: &mut String) {
    let marker = "bearer";
    let mut search_from = 0;
    while let Some(marker_start) = find_ascii_case_insensitive(value, marker, search_from) {
        let marker_end = marker_start + marker.len();
        if !is_word_boundary(value, marker_start, marker_end) {
            search_from = marker_end;
            continue;
        }
        let mut token_start = marker_end;
        while value[token_start..]
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_whitespace())
        {
            token_start += value[token_start..].chars().next().unwrap().len_utf8();
        }
        if token_start == marker_end || token_start == value.len() {
            search_from = marker_end;
            continue;
        }
        let token_end = value[token_start..]
            .find(|character: char| {
                character.is_whitespace() || matches!(character, ',' | ';' | '"' | '\'' | '}' | ']')
            })
            .map(|offset| token_start + offset)
            .unwrap_or(value.len());
        if token_end == token_start {
            search_from = marker_end;
            continue;
        }
        value.replace_range(token_start..token_end, "[REDACTED]");
        search_from = token_start + "[REDACTED]".len();
    }
}

fn redacted_value_range(input: &str, start: usize) -> (usize, usize) {
    if start >= input.len() {
        return (start, start);
    }
    let quote = input[start..]
        .chars()
        .next()
        .filter(|character| matches!(character, '"' | '\'' | '`'));
    let value_start = if quote.is_some() { start + 1 } else { start };
    let end = input[value_start..]
        .find(|character: char| {
            if let Some(quote) = quote {
                character == quote
            } else {
                character.is_whitespace() || matches!(character, ',' | ';' | '}' | ']' | '"' | '\'')
            }
        })
        .map(|offset| value_start + offset)
        .unwrap_or(input.len());
    (value_start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_codes_are_stable_machine_readable_values() {
        let diagnostic = ActionableDiagnostic::error(
            DiagnosticCode::GitNotFound,
            "Git is required",
            "Git was not found.",
            "LunaScope uses Git worktrees for isolation.",
            ["Install Git for Windows.", "Retry the check."],
        );
        let value = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(value["code"], "GIT_NOT_FOUND");
        assert_eq!(value["howToFix"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn diagnostics_redact_headers_keys_and_bearer_tokens() {
        let input = r#"Authorization: Bearer live-token api_key="sk-secret" password=hunter2; secondary=Bearer another-token Cookie=session-cookie credentialValue=stored-value"#;
        let redacted = redact_sensitive_text(input);
        assert!(!redacted.contains("live-token"));
        assert!(!redacted.contains("another-token"));
        assert!(!redacted.contains("sk-secret"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("session-cookie"));
        assert!(!redacted.contains("stored-value"));
        assert!(redacted.matches("[REDACTED]").count() >= 3);
    }

    #[test]
    fn diagnostics_redact_mixed_case_tab_and_single_quoted_secrets() {
        let input =
            "BeArEr\ttab-token Authorization\t:\t'single-secret' x-api-key = `backtick-secret`";
        let redacted = redact_sensitive_text(input);
        assert!(!redacted.contains("tab-token"));
        assert!(!redacted.contains("single-secret"));
        assert!(!redacted.contains("backtick-secret"));
    }

    #[test]
    fn explicit_error_codes_map_to_their_typed_diagnostics() {
        for (message, expected) in [
            (
                "DATA_ROOT_UNAVAILABLE: missing",
                DiagnosticCode::DataRootUnavailable,
            ),
            (
                "DATA_ROOT_NOT_WRITABLE: denied",
                DiagnosticCode::DataRootNotWritable,
            ),
            ("GIT_NOT_FOUND", DiagnosticCode::GitNotFound),
            (
                "PROVIDER_NOT_CONFIGURED",
                DiagnosticCode::ProviderNotConfigured,
            ),
            ("PROVIDER_AUTH_FAILED", DiagnosticCode::ProviderAuthFailed),
            (
                "PROVIDER_MODEL_UNSUPPORTED",
                DiagnosticCode::ProviderModelUnsupported,
            ),
            (
                "MODEL_REASONING_TEST_REQUIRED",
                DiagnosticCode::ModelReasoningTestRequired,
            ),
            ("WORKSPACE_INVALID", DiagnosticCode::WorkspaceInvalid),
            (
                "WORKSPACE_PERMISSION_DENIED",
                DiagnosticCode::WorkspacePermissionDenied,
            ),
            ("BROWSER_NOT_AVAILABLE", DiagnosticCode::BrowserNotAvailable),
            (
                "TOOL_DEPENDENCY_MISSING",
                DiagnosticCode::ToolDependencyMissing,
            ),
            ("RUN_INTERRUPTED", DiagnosticCode::RunInterrupted),
            (
                "RECOVERY_NEEDS_INTERVENTION",
                DiagnosticCode::RecoveryNeedsIntervention,
            ),
        ] {
            assert_eq!(classify_error(message).code, expected, "{message}");
        }
    }

    #[test]
    fn classifier_returns_actionable_recovery_diagnostic() {
        let diagnostic = classify_error("RECOVERY_NEEDS_INTERVENTION: apply_patch uncertain");
        assert_eq!(diagnostic.code, DiagnosticCode::RecoveryNeedsIntervention);
        assert!(diagnostic.why.contains("replay"));
        assert!(!diagnostic.how_to_fix.is_empty());
    }
}
