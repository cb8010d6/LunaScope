use std::{collections::HashMap, path::Path, time::Duration};

use http::{HeaderName, HeaderValue};
use lunascope_core::{
    McpConfigValue, McpHeader, McpInvocationResult, McpServerConfig, McpServerStatus,
    McpToolDescriptor, McpTransportConfig,
};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientInfo, ServerInfo, Tool},
    transport::{
        StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::McpCredentialStore;

const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1000;

pub struct NativeMcpClient<'a> {
    credentials: &'a McpCredentialStore,
}

impl<'a> NativeMcpClient<'a> {
    pub fn new(credentials: &'a McpCredentialStore) -> Self {
        Self { credentials }
    }

    pub async fn health_check(
        &self,
        config: &McpServerConfig,
        cancellation: CancellationToken,
    ) -> Result<McpServerStatus, McpError> {
        validate_mcp_config(config)?;
        let timeout = Duration::from_millis(config.timeout_ms);
        bounded(timeout, cancellation, async {
            match &config.transport {
                McpTransportConfig::Stdio {
                    program,
                    args,
                    cwd,
                    environment,
                } => {
                    let mut command = tokio::process::Command::new(program);
                    crate::hide_tokio_console_window(&mut command);
                    command.args(args);
                    command.kill_on_drop(true);
                    if let Some(cwd) = cwd {
                        command.current_dir(cwd);
                    }
                    command.env_clear();
                    for (name, value) in environment {
                        command.env(name, resolve_value(self.credentials, &config.id, value)?);
                    }
                    let transport = TokioChildProcess::new(command)?;
                    let client = ClientInfo::default()
                        .serve(transport)
                        .await
                        .map_err(protocol_error)?;
                    let tools = client.list_all_tools().await.map_err(protocol_error)?;
                    let status =
                        status_from_parts(&config.id, client.peer_info().as_deref(), &tools);
                    client.cancel().await.map_err(join_error)?;
                    Ok(status)
                }
                McpTransportConfig::StreamableHttp { url, headers } => {
                    let transport_config =
                        http_transport_config(self.credentials, &config.id, url, headers)?;
                    let transport = StreamableHttpClientTransport::from_config(transport_config);
                    let client = ClientInfo::default()
                        .serve(transport)
                        .await
                        .map_err(protocol_error)?;
                    let tools = client.list_all_tools().await.map_err(protocol_error)?;
                    let status =
                        status_from_parts(&config.id, client.peer_info().as_deref(), &tools);
                    client.cancel().await.map_err(join_error)?;
                    Ok(status)
                }
            }
        })
        .await
    }

    pub async fn invoke(
        &self,
        config: &McpServerConfig,
        tool_name: &str,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> Result<McpInvocationResult, McpError> {
        validate_mcp_config(config)?;
        if tool_name.trim().is_empty() {
            return Err(McpError::InvalidToolName);
        }
        let arguments = arguments
            .as_object()
            .cloned()
            .ok_or(McpError::ArgumentsNotObject)?;
        let timeout = Duration::from_millis(config.timeout_ms);
        let started = std::time::Instant::now();
        bounded(timeout, cancellation, async {
            let result = match &config.transport {
                McpTransportConfig::Stdio {
                    program,
                    args,
                    cwd,
                    environment,
                } => {
                    let mut command = tokio::process::Command::new(program);
                    crate::hide_tokio_console_window(&mut command);
                    command.args(args);
                    command.kill_on_drop(true);
                    if let Some(cwd) = cwd {
                        command.current_dir(cwd);
                    }
                    command.env_clear();
                    for (name, value) in environment {
                        command.env(name, resolve_value(self.credentials, &config.id, value)?);
                    }
                    let transport = TokioChildProcess::new(command)?;
                    let client = ClientInfo::default()
                        .serve(transport)
                        .await
                        .map_err(protocol_error)?;
                    let result = client
                        .call_tool(
                            CallToolRequestParams::new(tool_name.to_owned())
                                .with_arguments(arguments.clone()),
                        )
                        .await
                        .map_err(protocol_error)?;
                    client.cancel().await.map_err(join_error)?;
                    result
                }
                McpTransportConfig::StreamableHttp { url, headers } => {
                    let transport_config =
                        http_transport_config(self.credentials, &config.id, url, headers)?;
                    let transport = StreamableHttpClientTransport::from_config(transport_config);
                    let client = ClientInfo::default()
                        .serve(transport)
                        .await
                        .map_err(protocol_error)?;
                    let result = client
                        .call_tool(
                            CallToolRequestParams::new(tool_name.to_owned())
                                .with_arguments(arguments.clone()),
                        )
                        .await
                        .map_err(protocol_error)?;
                    client.cancel().await.map_err(join_error)?;
                    result
                }
            };
            Ok(McpInvocationResult {
                content: json!({
                    "content": result.content,
                    "structuredContent": result.structured_content,
                    "_meta": result.meta,
                }),
                is_error: result.is_error.unwrap_or(false),
                duration_ms: started.elapsed().as_millis() as u64,
            })
        })
        .await
    }
}

pub fn validate_mcp_config(config: &McpServerConfig) -> Result<(), McpError> {
    validate_identifier("server", &config.id)?;
    if config.name.trim().is_empty() {
        return Err(McpError::InvalidName);
    }
    if config.timeout_ms == 0 || config.timeout_ms > MAX_TIMEOUT_MS {
        return Err(McpError::InvalidTimeout);
    }
    match &config.transport {
        McpTransportConfig::Stdio {
            program,
            args: _,
            cwd,
            environment,
        } => {
            let program = Path::new(program);
            if !program.is_absolute() {
                return Err(McpError::ProgramNotAbsolute);
            }
            if let Some(cwd) = cwd
                && !Path::new(cwd).is_absolute()
            {
                return Err(McpError::WorkingDirectoryNotAbsolute);
            }
            for (name, value) in environment {
                if !valid_environment_name(name) {
                    return Err(McpError::InvalidEnvironmentName(name.clone()));
                }
                validate_config_value(value)?;
            }
        }
        McpTransportConfig::StreamableHttp { url, headers } => {
            let parsed = reqwest::Url::parse(url)?;
            if parsed.scheme() != "https" && !is_loopback(&parsed) {
                return Err(McpError::InsecureRemoteHttp);
            }
            for header in headers {
                validate_header(header)?;
            }
        }
    }
    Ok(())
}

fn status_from_parts(
    config_id: &str,
    server_info: Option<&ServerInfo>,
    tools: &[Tool],
) -> McpServerStatus {
    McpServerStatus {
        server_config_id: config_id.into(),
        healthy: true,
        server_name: server_info.map(|info| info.server_info.name.clone()),
        server_version: server_info.map(|info| info.server_info.version.clone()),
        tools: tools
            .iter()
            .map(|tool| McpToolDescriptor {
                server_config_id: config_id.into(),
                name: tool.name.to_string(),
                title: tool.title.clone(),
                description: tool.description.as_ref().map(ToString::to_string),
                input_schema: Value::Object((*tool.input_schema).clone()),
                output_schema: tool
                    .output_schema
                    .as_ref()
                    .map(|schema| Value::Object((**schema).clone())),
            })
            .collect(),
        error: None,
    }
}

fn http_transport_config(
    credentials: &McpCredentialStore,
    server_id: &str,
    url: &str,
    headers: &[McpHeader],
) -> Result<StreamableHttpClientTransportConfig, McpError> {
    let mut custom_headers = HashMap::new();
    for header in headers {
        let name = HeaderName::from_bytes(header.name.as_bytes())?;
        let value = resolve_value(credentials, server_id, &header.value)?;
        custom_headers.insert(name, HeaderValue::from_str(&value)?);
    }
    Ok(
        StreamableHttpClientTransportConfig::with_uri(url.to_owned())
            .custom_headers(custom_headers)
            .reinit_on_expired_session(true),
    )
}

fn validate_header(header: &McpHeader) -> Result<(), McpError> {
    let name = HeaderName::from_bytes(header.name.as_bytes())?;
    validate_config_value(&header.value)?;
    if name == http::header::AUTHORIZATION && matches!(header.value, McpConfigValue::Literal(_)) {
        return Err(McpError::SensitiveLiteralHeader);
    }
    if let McpConfigValue::Literal(value) = &header.value {
        HeaderValue::from_str(value)?;
    }
    Ok(())
}

fn validate_config_value(value: &McpConfigValue) -> Result<(), McpError> {
    match value {
        McpConfigValue::Literal(_) => Ok(()),
        McpConfigValue::CredentialReference(reference) => {
            validate_identifier("credential reference", reference)
        }
    }
}

fn resolve_value(
    credentials: &McpCredentialStore,
    server_id: &str,
    value: &McpConfigValue,
) -> Result<String, McpError> {
    match value {
        McpConfigValue::Literal(value) => Ok(value.clone()),
        McpConfigValue::CredentialReference(reference) => {
            Ok(credentials.get(server_id, reference)?.expose().to_owned())
        }
    }
}

fn validate_identifier(label: &'static str, value: &str) -> Result<(), McpError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(McpError::InvalidIdentifier {
            label,
            value: value.into(),
        })
    }
}

fn valid_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
}

fn is_loopback(url: &reqwest::Url) -> bool {
    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

async fn bounded<T, F>(
    timeout: Duration,
    cancellation: CancellationToken,
    operation: F,
) -> Result<T, McpError>
where
    F: Future<Output = Result<T, McpError>>,
{
    tokio::select! {
        _ = cancellation.cancelled() => Err(McpError::Cancelled),
        result = tokio::time::timeout(timeout, operation) => {
            result.map_err(|_| McpError::Timeout(timeout))?
        }
    }
}

fn protocol_error(error: impl std::fmt::Display) -> McpError {
    McpError::Protocol(error.to_string())
}

fn join_error(error: impl std::fmt::Display) -> McpError {
    McpError::Shutdown(error.to_string())
}

#[derive(Debug, Error)]
pub enum McpError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    InvalidHeaderName(#[from] http::header::InvalidHeaderName),
    #[error(transparent)]
    InvalidHeaderValue(#[from] http::header::InvalidHeaderValue),
    #[error(transparent)]
    Credential(#[from] crate::McpCredentialError),
    #[error("{label} identifier is invalid: {value}")]
    InvalidIdentifier { label: &'static str, value: String },
    #[error("MCP server name cannot be empty")]
    InvalidName,
    #[error("MCP timeout must be between 1 ms and 10 minutes")]
    InvalidTimeout,
    #[error("stdio MCP program must be an absolute path")]
    ProgramNotAbsolute,
    #[error("stdio MCP working directory must be absolute")]
    WorkingDirectoryNotAbsolute,
    #[error("invalid MCP environment variable name: {0}")]
    InvalidEnvironmentName(String),
    #[error("non-loopback streamable HTTP endpoints must use HTTPS")]
    InsecureRemoteHttp,
    #[error("sensitive HTTP headers must use a credential reference")]
    SensitiveLiteralHeader,
    #[error("MCP tool name cannot be empty")]
    InvalidToolName,
    #[error("MCP tool arguments must be a JSON object")]
    ArgumentsNotObject,
    #[error("MCP operation was cancelled")]
    Cancelled,
    #[error("MCP operation timed out after {0:?}")]
    Timeout(Duration),
    #[error("MCP protocol error: {0}")]
    Protocol(String),
    #[error("MCP shutdown error: {0}")]
    Shutdown(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn validates_transport_security_boundaries() {
        let remote_http = McpServerConfig {
            id: "remote".into(),
            name: "Remote".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "http://example.com/mcp".into(),
                headers: Vec::new(),
            },
            timeout_ms: 30_000,
            enabled: true,
        };
        assert!(matches!(
            validate_mcp_config(&remote_http),
            Err(McpError::InsecureRemoteHttp)
        ));

        let relative_program = McpServerConfig {
            id: "stdio".into(),
            name: "Stdio".into(),
            transport: McpTransportConfig::Stdio {
                program: "node".into(),
                args: Vec::new(),
                cwd: None,
                environment: BTreeMap::new(),
            },
            timeout_ms: 30_000,
            enabled: true,
        };
        assert!(matches!(
            validate_mcp_config(&relative_program),
            Err(McpError::ProgramNotAbsolute)
        ));
    }

    #[test]
    fn rejects_literal_authorization_but_accepts_keyring_reference() {
        let literal = McpHeader {
            name: "Authorization".into(),
            value: McpConfigValue::Literal("Bearer secret".into()),
        };
        assert!(matches!(
            validate_header(&literal),
            Err(McpError::SensitiveLiteralHeader)
        ));
        validate_header(&McpHeader {
            name: "Authorization".into(),
            value: McpConfigValue::CredentialReference("auth".into()),
        })
        .expect("credential reference header");
    }
}
