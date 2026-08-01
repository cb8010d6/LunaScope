use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use lunascope_core::{McpConfigValue, McpServerConfig, McpTransportConfig};
use lunascope_extensions::{McpCredentialStore, McpError, NativeMcpClient};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

fn powershell_path() -> PathBuf {
    PathBuf::from(std::env::var("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe")
}

fn stdio_config(script: &std::path::Path) -> McpServerConfig {
    let mut environment = BTreeMap::new();
    environment.insert(
        "SystemRoot".into(),
        McpConfigValue::Literal(std::env::var("SystemRoot").expect("SystemRoot")),
    );
    McpServerConfig {
        id: "stdio-fixture".into(),
        name: "Stdio fixture".into(),
        transport: McpTransportConfig::Stdio {
            program: powershell_path().to_string_lossy().into_owned(),
            args: vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script.to_string_lossy().into_owned(),
            ],
            cwd: script
                .parent()
                .map(|path| path.to_string_lossy().into_owned()),
            environment,
        },
        timeout_ms: 10_000,
        enabled: true,
    }
}

#[tokio::test]
async fn stdio_client_initializes_discovers_and_calls_tool() {
    let temp = tempfile::tempdir().expect("temp");
    let script = temp.path().join("mcp-fixture.ps1");
    std::fs::write(
        &script,
        r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$input | ForEach-Object {
  $message = $_ | ConvertFrom-Json
  if ($message.method -eq 'initialize') {
    $result = @{
      jsonrpc = '2.0'
      id = $message.id
      result = @{
        protocolVersion = $message.params.protocolVersion
        capabilities = @{ tools = @{} }
        serverInfo = @{ name = 'powershell-fixture'; version = '1.0.0' }
      }
    }
    [Console]::Out.WriteLine(($result | ConvertTo-Json -Compress -Depth 20))
  } elseif ($message.method -eq 'tools/list') {
    $result = @{
      jsonrpc = '2.0'
      id = $message.id
      result = @{
        tools = @(@{
          name = 'echo'
          title = 'Echo'
          description = 'Echo text'
          inputSchema = @{ type = 'object'; properties = @{ text = @{ type = 'string' } } }
        })
      }
    }
    [Console]::Out.WriteLine(($result | ConvertTo-Json -Compress -Depth 20))
  } elseif ($message.method -eq 'tools/call') {
    $text = $message.params.arguments.text
    $result = @{
      jsonrpc = '2.0'
      id = $message.id
      result = @{
        content = @(@{ type = 'text'; text = $text })
        isError = $false
      }
    }
    [Console]::Out.WriteLine(($result | ConvertTo-Json -Compress -Depth 20))
  }
  [Console]::Out.Flush()
}
"#,
    )
    .expect("write fixture");

    let credentials = McpCredentialStore;
    let client = NativeMcpClient::new(&credentials);
    let config = stdio_config(&script);
    let status = client
        .health_check(&config, CancellationToken::new())
        .await
        .expect("health check");
    assert!(status.healthy);
    assert_eq!(status.server_name.as_deref(), Some("powershell-fixture"));
    assert_eq!(status.tools.len(), 1);
    assert_eq!(status.tools[0].name, "echo");

    let result = client
        .invoke(
            &config,
            "echo",
            json!({"text": "stdio-ok"}),
            CancellationToken::new(),
        )
        .await
        .expect("invoke");
    assert!(!result.is_error);
    assert!(result.content.to_string().contains("stdio-ok"));
}

#[tokio::test]
async fn streamable_http_client_initializes_and_discovers_tools() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let request = read_http_request(&mut stream).await.expect("request");
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if method == "initialize" {
                let protocol = request["params"]["protocolVersion"].clone();
                write_json_response(
                    &mut stream,
                    json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": {
                            "protocolVersion": protocol,
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "http-fixture", "version": "1.0.0"}
                        }
                    }),
                )
                .await
                .expect("initialize response");
            } else if method == "tools/list" {
                write_json_response(
                    &mut stream,
                    json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": {
                            "tools": [{
                                "name": "lookup",
                                "description": "Look up a value",
                                "inputSchema": {"type": "object"}
                            }]
                        }
                    }),
                )
                .await
                .expect("tools response");
            } else {
                write_empty_response(&mut stream)
                    .await
                    .expect("notification");
            }
        }
    });

    let config = McpServerConfig {
        id: "http-fixture".into(),
        name: "HTTP fixture".into(),
        transport: McpTransportConfig::StreamableHttp {
            url: format!("http://{address}/mcp"),
            headers: Vec::new(),
        },
        timeout_ms: 10_000,
        enabled: true,
    };
    let credentials = McpCredentialStore;
    let status = NativeMcpClient::new(&credentials)
        .health_check(&config, CancellationToken::new())
        .await
        .expect("health");
    assert_eq!(status.server_name.as_deref(), Some("http-fixture"));
    assert_eq!(status.tools[0].name, "lookup");
    server.await.expect("server");
}

#[tokio::test]
async fn cancellation_interrupts_a_hung_streamable_http_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let reached_tools = Arc::new(tokio::sync::Notify::new());
    let reached_tools_server = reached_tools.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let request = read_http_request(&mut stream).await.expect("request");
            match request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "initialize" => {
                    write_json_response(
                        &mut stream,
                        json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "result": {
                                "protocolVersion": request["params"]["protocolVersion"],
                                "capabilities": {"tools": {}},
                                "serverInfo": {"name": "hung", "version": "1"}
                            }
                        }),
                    )
                    .await
                    .expect("initialize");
                }
                "tools/list" => {
                    reached_tools_server.notify_one();
                    std::future::pending::<()>().await;
                }
                _ => write_empty_response(&mut stream)
                    .await
                    .expect("notification"),
            }
        }
    });
    let config = McpServerConfig {
        id: "hung".into(),
        name: "Hung".into(),
        transport: McpTransportConfig::StreamableHttp {
            url: format!("http://{address}/mcp"),
            headers: Vec::new(),
        },
        timeout_ms: 10_000,
        enabled: true,
    };
    let cancellation = CancellationToken::new();
    let cancellation_for_task = cancellation.clone();
    let credentials = McpCredentialStore;
    let operation = async {
        NativeMcpClient::new(&credentials)
            .health_check(&config, cancellation_for_task)
            .await
    };
    let cancel = async {
        reached_tools.notified().await;
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(operation, cancel);
    assert!(matches!(result, Err(McpError::Cancelled)));
    server.abort();
}

async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<Value> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        header.push(byte[0]);
        if header.len() > 32 * 1024 {
            return Err(std::io::Error::other("HTTP header too large"));
        }
    }
    let header_text = String::from_utf8_lossy(&header);
    let content_length = header_text
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    let mut body = vec![0; content_length];
    stream.read_exact(&mut body).await?;
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}

async fn write_json_response(stream: &mut TcpStream, body: Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(&body).map_err(std::io::Error::other)?;
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await?;
    stream.write_all(&body).await?;
    stream.flush().await
}

async fn write_empty_response(stream: &mut TcpStream) -> std::io::Result<()> {
    stream
        .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n")
        .await?;
    stream.flush().await
}
