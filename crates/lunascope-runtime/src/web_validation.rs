use std::{
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::Command,
};
use tokio_util::sync::CancellationToken;

use crate::installed_headless_browser;

const MAX_FIXTURE_FILES: usize = 2_000;
const MAX_FIXTURE_BYTES: u64 = 96 * 1024 * 1024;
const MAX_BROWSER_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_BROWSER_ACTIONS: usize = 48;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAction {
    pub selector: String,
    pub action: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct BrowserCheckRequest {
    pub workspace_root: PathBuf,
    pub relative_path: String,
    pub actions: Vec<BrowserAction>,
    pub query: Option<String>,
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f32,
    pub virtual_time_ms: u64,
    pub capture_screenshot: bool,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCheckReport {
    pub browser: String,
    pub page: String,
    pub url: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub action_count: usize,
    pub dom: String,
    pub diagnostics: String,
    pub runtime_report: serde_json::Value,
    pub screenshot_path: Option<String>,
}

pub async fn check_browser_page(
    request: BrowserCheckRequest,
) -> Result<BrowserCheckReport, String> {
    validate_relative_path(&request.relative_path)?;
    if request.actions.len() > MAX_BROWSER_ACTIONS {
        return Err(format!(
            "browser actions exceed the {MAX_BROWSER_ACTIONS}-action limit"
        ));
    }
    validate_actions(&request.actions)?;
    let workspace = request
        .workspace_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !workspace.is_dir() {
        return Err("browser workspace is not a directory".into());
    }
    let page = resolve_existing(&workspace, &request.relative_path)?;
    if !page.is_file()
        || !page
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("html"))
    {
        return Err("browser check requires an existing workspace-local .html file".into());
    }
    let browser = installed_headless_browser()
        .ok_or_else(|| "no supported local Edge or Chrome executable was found".to_owned())?;
    let temporary = tempfile::Builder::new()
        .prefix("lunascope-web-validation-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let content_root = temporary.path().join("content");
    copy_fixture_tree(&workspace, &content_root)?;
    let instrumented_page = content_root.join(&request.relative_path);
    inject_browser_probe(&instrumented_page, &request.actions)?;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let server_cancel = CancellationToken::new();
    let server_task = tokio::spawn(serve_static(
        listener,
        Arc::new(content_root.clone()),
        server_cancel.clone(),
    ));

    let encoded_path = request
        .relative_path
        .split('/')
        .map(percent_encode_path_segment)
        .collect::<Vec<_>>()
        .join("/");
    let mut page_url = format!("http://127.0.0.1:{port}/{encoded_path}");
    if let Some(query) = request.query.as_deref().filter(|query| !query.is_empty()) {
        page_url.push('?');
        page_url.push_str(query.trim_start_matches('?'));
    }
    let profile = temporary.path().join("profile");
    let temporary_screenshot = temporary.path().join("page.png");
    let mut command = Command::new(&browser);
    crate::hide_tokio_console_window(&mut command);
    command
        .args([
            "--headless=new",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-default-apps",
            "--disable-extensions",
            "--disable-sync",
            "--metrics-recording-only",
            "--no-default-browser-check",
            "--no-first-run",
            "--enable-logging=stderr",
            "--log-level=1",
            "--dump-dom",
            "--hide-scrollbars",
            "--proxy-server=127.0.0.1:9",
            "--proxy-bypass-list=127.0.0.1;localhost",
        ])
        .arg(format!(
            "--virtual-time-budget={}",
            request.virtual_time_ms.clamp(500, 30_000)
        ))
        .arg(format!(
            "--window-size={},{}",
            request.width.clamp(320, 3840),
            request.height.clamp(240, 2160)
        ))
        .arg(format!(
            "--force-device-scale-factor={}",
            request.device_scale_factor.clamp(1.0, 3.0)
        ))
        .arg(format!("--user-data-dir={}", profile.display()));
    if request.capture_screenshot {
        command.arg(format!("--screenshot={}", temporary_screenshot.display()));
    }
    command
        .arg(&page_url)
        .current_dir(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear();
    for name in [
        "PATH",
        "Path",
        "PATHEXT",
        "SYSTEMROOT",
        "SystemRoot",
        "SystemDrive",
        "ProgramData",
        "TEMP",
        "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let child = command.spawn().map_err(|error| error.to_string())?;
    let process_id = child.id();
    let output = child.wait_with_output();
    tokio::pin!(output);
    let output = tokio::select! {
        _ = request.cancellation.cancelled() => {
            terminate_process_tree(process_id).await;
            server_cancel.cancel();
            let _ = server_task.await;
            return Err("browser check cancelled".into());
        },
        _ = tokio::time::sleep(Duration::from_secs(45)) => {
            terminate_process_tree(process_id).await;
            server_cancel.cancel();
            let _ = server_task.await;
            return Err("browser check timed out".into());
        },
        result = &mut output => result.map_err(|error| error.to_string())?
    };
    server_cancel.cancel();
    let _ = server_task.await;

    let dom = bounded_text(&output.stdout);
    let diagnostics = bounded_text(&output.stderr);
    let runtime_report = extract_runtime_report(&dom).unwrap_or_else(|| {
        serde_json::json!({
            "status": "missing",
            "runtimeErrors": ["LunaScope browser probe did not produce a report"]
        })
    });
    let screenshot_path = if request.capture_screenshot && temporary_screenshot.is_file() {
        let evidence_dir = workspace.join(".lunascope/evidence/browser");
        std::fs::create_dir_all(&evidence_dir).map_err(|error| error.to_string())?;
        let name = format!("browser-{}.png", uuid::Uuid::new_v4());
        let destination = evidence_dir.join(&name);
        std::fs::copy(&temporary_screenshot, &destination).map_err(|error| error.to_string())?;
        Some(format!(".lunascope/evidence/browser/{name}"))
    } else {
        None
    };
    let runtime_ok = runtime_report
        .get("status")
        .and_then(serde_json::Value::as_str)
        == Some("passed")
        && runtime_report
            .get("runtimeErrors")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty)
        && runtime_report
            .get("consoleErrors")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty);
    Ok(BrowserCheckReport {
        browser: browser
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("browser")
            .into(),
        page: request.relative_path,
        url: page_url,
        exit_code: output.status.code(),
        success: output.status.success() && runtime_ok,
        action_count: request.actions.len(),
        dom,
        diagnostics,
        runtime_report,
        screenshot_path,
    })
}

fn validate_actions(actions: &[BrowserAction]) -> Result<(), String> {
    for action in actions {
        if action.selector.trim().is_empty() || action.selector.len() > 512 {
            return Err("browser action selector must contain at most 512 characters".into());
        }
        if !matches!(
            action.action.as_str(),
            "click" | "setValue" | "pressKey" | "pointerMove"
        ) {
            return Err(format!("unsupported browser action: {}", action.action));
        }
        if action.action == "pressKey" && action.value.as_deref().is_none_or(str::is_empty) {
            return Err("pressKey requires a value".into());
        }
    }
    Ok(())
}

fn inject_browser_probe(page: &Path, actions: &[BrowserAction]) -> Result<(), String> {
    let html = std::fs::read_to_string(page)
        .map_err(|error| format!("browser page is not UTF-8: {error}"))?;
    let actions_json = serde_json::to_vec(actions).map_err(|error| error.to_string())?;
    let actions_base64 = base64::engine::general_purpose::STANDARD.encode(actions_json);
    let probe = browser_probe_script(&actions_base64);
    let injected = if let Some(index) = html.to_ascii_lowercase().rfind("</body>") {
        format!("{}{}{}", &html[..index], probe, &html[index..])
    } else {
        format!("{html}{probe}")
    };
    std::fs::write(page, injected).map_err(|error| error.to_string())
}

fn browser_probe_script(actions_base64: &str) -> String {
    format!(
        r#"<script>(()=>{{
const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const report={{status:'running',consoleErrors:[],consoleWarnings:[],runtimeErrors:[],shaderDiagnostics:[],applicationSignals:[],actions:[],webgl:null,canvas:null,location:location.href}};
for(const level of ['error','warn']){{const original=console[level].bind(console);console[level]=(...args)=>{{const text=args.map(value=>String(value?.stack||value)).join(' ');(level==='error'?report.consoleErrors:report.consoleWarnings).push(text.slice(0,2000));original(...args);}};}}
addEventListener('error',event=>{{const target=event.target;const resource=target&&target!==window?(target.src||target.href||target.currentSrc||target.tagName):null;report.runtimeErrors.push(String(resource?`resource/module load failed: ${{resource}}`:event.error?.stack||event.message||'window error').slice(0,2000));}},true);
addEventListener('unhandledrejection',event=>report.runtimeErrors.push(String(event.reason?.stack||event.reason||'unhandled rejection').slice(0,2000)));
const instrumentedContexts=new WeakSet();
const instrumentGL=gl=>{{
  if(!gl||typeof gl.compileShader!=='function'||typeof gl.linkProgram!=='function'||instrumentedContexts.has(gl))return gl;
  instrumentedContexts.add(gl);
  try{{
    const compile=gl.compileShader.bind(gl);
    gl.compileShader=shader=>{{compile(shader);try{{if(!gl.getShaderParameter(shader,gl.COMPILE_STATUS))report.shaderDiagnostics.push({{severity:'error',stage:gl.getShaderParameter(shader,gl.SHADER_TYPE)===gl.VERTEX_SHADER?'vertex':'fragment',log:String(gl.getShaderInfoLog(shader)||'shader compilation failed').slice(0,4000),source:String(gl.getShaderSource(shader)||'').slice(0,4000)}});}}catch(error){{report.shaderDiagnostics.push({{severity:'warning',stage:'compile-probe',log:String(error?.message||error).slice(0,1000)}});}}}};
    const link=gl.linkProgram.bind(gl);
    gl.linkProgram=program=>{{link(program);try{{if(!gl.getProgramParameter(program,gl.LINK_STATUS))report.shaderDiagnostics.push({{severity:'error',stage:'link',log:String(gl.getProgramInfoLog(program)||'program link failed').slice(0,4000)}});}}catch(error){{report.shaderDiagnostics.push({{severity:'warning',stage:'link-probe',log:String(error?.message||error).slice(0,1000)}});}}}};
  }}catch(error){{report.shaderDiagnostics.push({{severity:'warning',stage:'instrumentation',log:String(error?.message||error).slice(0,1000)}});}}
  return gl;
}};
const originalGetContext=HTMLCanvasElement.prototype.getContext;
HTMLCanvasElement.prototype.getContext=function(...args){{return instrumentGL(originalGetContext.apply(this,args));}};
addEventListener('load',async()=>{{
  const output=document.createElement('output');output.id='lunascope-browser-report';output.hidden=true;document.body.append(output);
  try{{
    const bytes=Uint8Array.from(atob('{actions_base64}'),char=>char.charCodeAt(0));
    const actions=JSON.parse(new TextDecoder().decode(bytes));
    await wait(350);
    for(let index=0;index<actions.length;index++){{
      const action=actions[index];const element=document.querySelector(action.selector);
      if(!element)throw new Error(`selector not found: ${{action.selector}}`);
      if(action.action==='setValue'){{element.value=String(action.value??'');element.dispatchEvent(new Event('input',{{bubbles:true}}));element.dispatchEvent(new Event('change',{{bubbles:true}}));}}
      else if(action.action==='click')element.click();
      else if(action.action==='pressKey'){{element.focus();for(const type of ['keydown','keyup'])element.dispatchEvent(new KeyboardEvent(type,{{key:String(action.value??''),bubbles:true}}));}}
      else if(action.action==='pointerMove'){{const rect=element.getBoundingClientRect();element.dispatchEvent(new PointerEvent('pointermove',{{clientX:rect.left+rect.width/2,clientY:rect.top+rect.height/2,bubbles:true}}));}}
      report.actions.push({{index:index+1,action:action.action,selector:action.selector,status:'passed'}});
      await wait(Math.min(5000,Math.max(0,Number(action.waitMs??180))));
    }}
    await wait(400);
    const viewportArea=Math.max(1,innerWidth*innerHeight);
    const overlays=[...document.body.querySelectorAll('*')].map(element=>{{const style=getComputedStyle(element);const rect=element.getBoundingClientRect();const area=Math.max(0,Math.min(innerWidth,rect.right)-Math.max(0,rect.left))*Math.max(0,Math.min(innerHeight,rect.bottom)-Math.max(0,rect.top));return {{element,style,rect,ratio:area/viewportArea}};}}).filter(item=>item.element.tagName!=='CANVAS'&&['fixed','sticky'].includes(item.style.position)&&item.style.visibility!=='hidden'&&item.style.display!=='none'&&item.ratio>.04).sort((a,b)=>b.ratio-a.ratio);
    const largestOverlay=overlays[0];
    report.layout={{viewportWidth:innerWidth,viewportHeight:innerHeight,documentWidth:document.documentElement.scrollWidth,documentHeight:document.documentElement.scrollHeight,horizontalOverflow:document.documentElement.scrollWidth>innerWidth+2,largestFixedOverlayRatio:Number((largestOverlay?.ratio||0).toFixed(4)),largestFixedOverlay:largestOverlay?`${{largestOverlay.element.tagName.toLowerCase()}}${{largestOverlay.element.id?'#'+largestOverlay.element.id:''}}${{largestOverlay.element.classList.length?'.'+[...largestOverlay.element.classList].slice(0,3).join('.'):''}}`:null}};
    const canvas=document.querySelector('canvas');
    if(canvas){{
      const rect=canvas.getBoundingClientRect();const visibleWidth=Math.max(0,Math.min(innerWidth,rect.right)-Math.max(0,rect.left)),visibleHeight=Math.max(0,Math.min(innerHeight,rect.bottom)-Math.max(0,rect.top));report.canvas={{width:canvas.width,height:canvas.height,clientWidth:Math.round(rect.width),clientHeight:Math.round(rect.height),visible:rect.width>0&&rect.height>0,viewportCoverage:Number(((visibleWidth*visibleHeight)/viewportArea).toFixed(4))}};
      const gl=canvas.getContext('webgl2')||canvas.getContext('webgl')||canvas.getContext('experimental-webgl');
      report.webgl=gl?{{supported:true,version:String(gl.getParameter(gl.VERSION)),renderer:String(gl.getParameter(gl.RENDERER)),vendor:String(gl.getParameter(gl.VENDOR)),contextLost:Boolean(gl.isContextLost?.())}}:{{supported:false}};
      try{{
        const sample=document.createElement('canvas');sample.width=64;sample.height=64;
        const context=sample.getContext('2d',{{willReadFrequently:true}});context.drawImage(canvas,0,0,64,64);
        const pixels=context.getImageData(0,0,64,64).data;let nonBlack=0,nonTransparent=0,dark=0,bright=0,nearWhite=0,saturated=0,sum=0,sumSquares=0;const colors=new Set(),luminances=[];
        for(let index=0;index<pixels.length;index+=4){{const r=pixels[index],g=pixels[index+1],b=pixels[index+2],a=pixels[index+3];const luminance=.2126*r+.7152*g+.0722*b;luminances.push(luminance);if(a>0)nonTransparent++;if(luminance>4)nonBlack++;if(luminance<15)dark++;if(luminance>180)bright++;if(luminance>245)nearWhite++;if(Math.max(r,g,b)>=252)saturated++;sum+=luminance;sumSquares+=luminance*luminance;colors.add(`${{r>>4}},${{g>>4}},${{b>>4}},${{a>>4}}`);}}
        const count=pixels.length/4,mean=sum/count,variance=Math.max(0,sumSquares/count-mean*mean);luminances.sort((a,b)=>a-b);const percentile=value=>Number(luminances[Math.min(count-1,Math.max(0,Math.floor((count-1)*value)))].toFixed(2));
        const visualSample={{sampleWidth:64,sampleHeight:64,nonBlackPixels:nonBlack,nonTransparentPixels:nonTransparent,darkPixels:dark,brightPixels:bright,nearWhitePixels:nearWhite,saturatedPixels:saturated,darkRatio:Number((dark/count).toFixed(4)),brightRatio:Number((bright/count).toFixed(4)),nearWhiteRatio:Number((nearWhite/count).toFixed(4)),saturatedRatio:Number((saturated/count).toFixed(4)),uniqueQuantizedColors:colors.size,meanLuminance:Number(mean.toFixed(2)),luminanceStdDev:Number(Math.sqrt(variance).toFixed(2)),p10Luminance:percentile(.1),medianLuminance:percentile(.5),p90Luminance:percentile(.9),likelyBlank:nonTransparent===0||(nonBlack<4&&colors.size<3)}};
        visualSample.qualitySignals={{lowDynamicRangeLikely:visualSample.luminanceStdDev<12,clippedHighlightsLikely:visualSample.nearWhiteRatio>.45||visualSample.saturatedRatio>.65,lacksDarkRangeLikely:visualSample.darkRatio<.005,mobileViewportDominatedByOverlay:innerWidth<=600&&report.layout.largestFixedOverlayRatio>.72,horizontalOverflow:report.layout.horizontalOverflow}};
        report.canvas.visualSample=visualSample;
      }}catch(error){{report.canvas.visualSample={{error:String(error?.message||error).slice(0,500)}};}}
    }}
    for(const meta of [...document.querySelectorAll('meta[name]')].filter(item=>/(lunascope|verify|status|test|debug)/i.test(item.name)).slice(0,24)){{report.applicationSignals.push({{source:'meta',name:String(meta.name).slice(0,160),value:String(meta.content||'').slice(0,4000)}});}}
    for(const element of [...document.querySelectorAll('[data-lunascope-signal]')].slice(0,24)){{report.applicationSignals.push({{source:'element',name:String(element.getAttribute('data-lunascope-signal')||element.id||element.tagName).slice(0,160),value:String(element.value??element.textContent??'').trim().slice(0,4000)}});}}
    if(window.__lunascopeDiagnostics!==undefined){{try{{report.applicationSignals.push({{source:'window',name:'__lunascopeDiagnostics',value:JSON.parse(JSON.stringify(window.__lunascopeDiagnostics))}});}}catch(error){{report.applicationSignals.push({{source:'window',name:'__lunascopeDiagnostics',value:String(window.__lunascopeDiagnostics).slice(0,4000)}});}}}}
    const blankCanvas=Boolean(report.canvas?.visualSample?.likelyBlank);
    const shaderFailed=report.shaderDiagnostics.some(item=>item.severity==='error');
    report.status=report.runtimeErrors.length||report.consoleErrors.length||shaderFailed||blankCanvas?'failed':'passed';
  }}catch(error){{report.runtimeErrors.push(String(error?.stack||error).slice(0,2000));report.status='failed';}}
  output.textContent=btoa(unescape(encodeURIComponent(JSON.stringify(report))));output.dataset.status=report.status;
}});
}})();</script>"#
    )
}

fn extract_runtime_report(dom: &str) -> Option<serde_json::Value> {
    let marker = "id=\"lunascope-browser-report\"";
    let start = dom.find(marker)?;
    let content_start = dom[start..].find('>')? + start + 1;
    let content_end = dom[content_start..].find("</output>")? + content_start;
    let encoded = dom[content_start..content_end].trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn serve_static(listener: TcpListener, root: Arc<PathBuf>, cancel: CancellationToken) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            incoming = listener.accept() => {
                let Ok((stream, _)) = incoming else { break };
                let root = Arc::clone(&root);
                tokio::spawn(async move { let _ = serve_connection(stream, &root).await; });
            }
        }
    }
}

async fn serve_connection(mut stream: TcpStream, root: &Path) -> Result<(), String> {
    let mut request = vec![0_u8; 16 * 1024];
    let read = stream
        .read(&mut request)
        .await
        .map_err(|error| error.to_string())?;
    request.truncate(read);
    let first_line = String::from_utf8_lossy(&request)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    let mut parts = first_line.split_whitespace();
    if parts.next() != Some("GET") {
        return write_response(&mut stream, 405, "text/plain", b"method not allowed").await;
    }
    let request_target = parts.next().unwrap_or("/");
    let raw_path = request_target.split('?').next().unwrap_or("/");
    let decoded = percent_decode_path(raw_path.trim_start_matches('/'))?;
    let relative = if decoded.is_empty() {
        "index.html"
    } else {
        &decoded
    };
    validate_relative_path(relative)?;
    let path = resolve_existing(root, relative)?;
    if !path.is_file() {
        return write_response(&mut stream, 404, "text/plain", b"not found").await;
    }
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    write_response(&mut stream, 200, content_type(&path), &bytes).await
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(body)
        .await
        .map_err(|error| error.to_string())
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn copy_fixture_tree(source: &Path, target: &Path) -> Result<(), String> {
    fn copy_directory(
        root: &Path,
        current: &Path,
        target: &Path,
        files: &mut usize,
        bytes: &mut u64,
    ) -> Result<(), String> {
        let mut entries = std::fs::read_dir(current)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
            if relative.components().any(|component| {
                matches!(
                    component.as_os_str().to_str(),
                    Some(".git" | ".lunascope" | "target")
                ) || component.as_os_str() == "node_modules"
            }) {
                continue;
            }
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            let destination = target.join(relative);
            if file_type.is_dir() {
                std::fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
                copy_directory(root, &path, target, files, bytes)?;
            } else if file_type.is_file() {
                let length = entry.metadata().map_err(|error| error.to_string())?.len();
                *files += 1;
                *bytes = bytes.saturating_add(length);
                if *files > MAX_FIXTURE_FILES || *bytes > MAX_FIXTURE_BYTES {
                    return Err(format!(
                        "browser fixture exceeds {MAX_FIXTURE_FILES} files or {MAX_FIXTURE_BYTES} bytes"
                    ));
                }
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                std::fs::copy(&path, destination).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
    std::fs::create_dir_all(target).map_err(|error| error.to_string())?;
    let mut files = 0;
    let mut bytes = 0;
    copy_directory(source, source, target, &mut files, &mut bytes)
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe workspace-relative path: {value}"));
    }
    Ok(())
}

fn resolve_existing(root: &Path, relative: &str) -> Result<PathBuf, String> {
    validate_relative_path(relative)?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !path.starts_with(&root) {
        return Err(format!("path escaped the workspace: {relative}"));
    }
    Ok(path)
}

fn percent_encode_path_segment(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                vec![char::from(byte)]
            } else {
                format!("%{byte:02X}").chars().collect()
            }
        })
        .collect()
}

fn percent_decode_path(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("invalid percent-encoded browser path".into());
            }
            let digits = std::str::from_utf8(&bytes[index + 1..index + 3])
                .map_err(|_| "invalid browser path encoding")?;
            output.push(
                u8::from_str_radix(digits, 16)
                    .map_err(|_| "invalid percent-encoded browser path")?,
            );
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| "browser path is not UTF-8".into())
}

fn bounded_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BROWSER_OUTPUT_BYTES)]).into_owned()
}

async fn terminate_process_tree(process_id: Option<u32>) {
    let Some(process_id) = process_id else {
        return;
    };
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill.exe");
        crate::hide_tokio_console_window(&mut command);
        let _ = command
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &process_id.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_report_round_trips_from_dom() {
        let value = serde_json::json!({"status":"passed","runtimeErrors":[],"consoleErrors":[]});
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&value).expect("serialize"));
        let dom = format!(
            "<output id=\"lunascope-browser-report\" data-status=\"passed\">{encoded}</output>"
        );
        assert_eq!(extract_runtime_report(&dom), Some(value));
    }

    #[tokio::test]
    #[ignore = "requires an installed Edge or Chrome executable"]
    async fn real_browser_runs_es_modules_over_http_and_captures_visual_evidence() {
        let workspace = tempfile::tempdir().expect("workspace");
        std::fs::write(
            workspace.path().join("module.js"),
            "const canvas=document.querySelector('#view');const gl=canvas.getContext('webgl',{preserveDrawingBuffer:true});gl.clearColor(1,0,0,1);gl.clear(gl.COLOR_BUFFER_BIT);document.querySelector('#state').textContent='MODULE_OK';window.__lunascopeDiagnostics={phase:'rendered',pixels:'red'};",
        )
        .expect("module");
        std::fs::write(
            workspace.path().join("index.html"),
            "<!doctype html><canvas id='view' width='320' height='180'></canvas><button id='go'>Go</button><output id='state'>WAIT</output><script type='module'>import './module.js';document.querySelector('#go').addEventListener('click',()=>document.querySelector('#state').textContent='CLICK_OK')</script>",
        )
        .expect("page");
        let report = check_browser_page(BrowserCheckRequest {
            workspace_root: workspace.path().to_path_buf(),
            relative_path: "index.html".into(),
            actions: vec![BrowserAction {
                selector: "#go".into(),
                action: "click".into(),
                value: None,
                wait_ms: Some(100),
            }],
            query: None,
            width: 1280,
            height: 720,
            device_scale_factor: 1.0,
            virtual_time_ms: 3_000,
            capture_screenshot: true,
            cancellation: CancellationToken::new(),
        })
        .await
        .expect("browser report");
        assert!(report.success, "{report:#?}");
        assert!(report.dom.contains("CLICK_OK"));
        assert_eq!(report.runtime_report["webgl"]["supported"], true);
        assert!(
            report.runtime_report["shaderDiagnostics"]
                .as_array()
                .is_some_and(|items| items.iter().all(|item| item["severity"] != "error")),
            "unexpected shader failure: {report:#?}"
        );
        assert_eq!(
            report.runtime_report["applicationSignals"][0]["name"],
            "__lunascopeDiagnostics"
        );
        assert_eq!(
            report.runtime_report["applicationSignals"][0]["value"]["phase"],
            "rendered"
        );
        assert_eq!(
            report.runtime_report["canvas"]["visualSample"]["likelyBlank"],
            false
        );
        assert!(
            report.runtime_report["canvas"]["visualSample"]["nonBlackPixels"]
                .as_u64()
                .is_some_and(|count| count > 1_000)
        );
        assert!(
            workspace
                .path()
                .join(report.screenshot_path.expect("screenshot"))
                .is_file()
        );
    }

    #[test]
    fn browser_probe_instruments_shader_and_application_diagnostics() {
        let probe = browser_probe_script("");
        assert!(probe.contains("compileShader"));
        assert!(probe.contains("getShaderInfoLog"));
        assert!(probe.contains("linkProgram"));
        assert!(probe.contains("__lunascopeDiagnostics"));
        assert!(probe.contains("data-lunascope-signal"));
    }

    #[tokio::test]
    #[ignore = "set LUNASCOPE_BROWSER_FIXTURE_ROOT to inspect a real local frontend fixture"]
    async fn reports_external_browser_fixture_quality_signals() {
        let workspace = std::env::var_os("LUNASCOPE_BROWSER_FIXTURE_ROOT")
            .map(PathBuf::from)
            .expect("LUNASCOPE_BROWSER_FIXTURE_ROOT");
        let report = check_browser_page(BrowserCheckRequest {
            workspace_root: workspace,
            relative_path: "index.html".into(),
            actions: Vec::new(),
            query: None,
            width: std::env::var("LUNASCOPE_BROWSER_FIXTURE_WIDTH")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(390),
            height: 844,
            device_scale_factor: 2.0,
            virtual_time_ms: 8_000,
            capture_screenshot: true,
            cancellation: CancellationToken::new(),
        })
        .await
        .expect("browser fixture report");
        println!(
            "{}",
            serde_json::to_string_pretty(&report.runtime_report).expect("serialize report")
        );
        assert!(report.success, "{report:#?}");
    }
}
