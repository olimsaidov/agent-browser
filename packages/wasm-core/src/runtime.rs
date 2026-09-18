use std::cell::RefCell;
use std::collections::HashMap;

use js_sys::{Function, Promise, WeakMap};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::commands;
use crate::connection::Response;
use crate::flags::{self, Flags};
use crate::output::OutputOptions;
use crate::snapshot_format::{self, SnapshotRef};

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn monotonic_now() -> f64;
    #[wasm_bindgen(js_name = setTimeout)]
    fn set_timeout(callback: &Function, delay: f64);
}

thread_local! {
    static STATE: RefCell<WasmState> = RefCell::new(WasmState::default());
    static MOUSE_STATES: WeakMap = WeakMap::new();
}

#[derive(Default, Deserialize, Serialize)]
struct MouseState {
    x: f64,
    y: f64,
    buttons: u32,
}

#[derive(Default)]
struct WasmState {
    refs: HashMap<String, SnapshotRef>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
    clear: bool,
}

impl RunOutput {
    fn success(stdout: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: 0,
            clear: false,
        }
    }

    fn clear() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            clear: true,
        }
    }

    fn error(stderr: impl Into<String>) -> Self {
        Self {
            stdout: String::new(),
            stderr: stderr.into(),
            exit_code: 1,
            clear: false,
        }
    }
}

pub async fn run_command_line(input: &str, transport: Function) -> RunOutput {
    let input = input.trim();
    if input.is_empty() {
        return RunOutput::success("");
    }

    let parts = commands::shell_words_split(input);
    if parts.is_empty() {
        return RunOutput::success("");
    }

    match parts[0].as_str() {
        "clear" => return RunOutput::clear(),
        "agent-browser" => {}
        command => return RunOutput::error(format!("bash: command not found: {}", command)),
    }

    let args = parts[1..].to_vec();
    run_agent_args(args, transport).await
}

pub async fn run_command(input: &str, transport: Function) -> RunOutput {
    let input = input.trim();
    if input.is_empty() {
        return RunOutput::success(full_help_text());
    }

    run_agent_args(commands::shell_words_split(input), transport).await
}

pub async fn run_argv(args: Vec<String>, transport: Function) -> RunOutput {
    run_agent_args(args, transport).await
}

pub fn reset_state() {
    STATE.with(|state| {
        state.borrow_mut().refs.clear();
    });
}

async fn run_agent_args(args: Vec<String>, transport: Function) -> RunOutput {
    let flags = wasm_flags(&args);
    let clean = flags::clean_args(&args);

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        let stdout = clean
            .first()
            .and_then(|command| command_help_text(command))
            .unwrap_or_else(full_help_text);
        return RunOutput::success(stdout);
    }

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        return RunOutput::success(format!("agent-browser {}", env!("CARGO_PKG_VERSION")));
    }

    if clean.is_empty() {
        return RunOutput::success(full_help_text());
    }

    if let Some(message) = unavailable_top_level_command(&clean) {
        return RunOutput::error(message);
    }

    let mut command = match commands::parse_command(&clean, &flags) {
        Ok(command) => command,
        Err(error) => return RunOutput::error(error.format()),
    };
    if !matches!(flags.input_mode.as_str(), "instant" | "smooth" | "human") {
        return RunOutput::error("--input-mode must be instant, smooth, or human");
    }
    if command.get("inputMode").is_none() {
        command["inputMode"] = json!(flags.input_mode);
    }
    let action = command
        .get("action")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let response = execute(command, transport).await;
    format_response(&response, action.as_deref(), &flags)
}

fn unavailable_top_level_command(clean: &[String]) -> Option<String> {
    let command = clean.first()?.as_str();
    let message = match command {
        "install" => "Command is not available in the browser build: install",
        "upgrade" => "Command is not available in the browser build: upgrade",
        "doctor" => "Command is not available in the browser build: doctor",
        "dashboard" => "Command is not available in the browser build: dashboard",
        "profiles" => "Command is not available in the browser build: profiles",
        "skills" => "Command is not available in the browser build: skills",
        "session" => "Command is not available in the browser build: session",
        "chat" => "Command is not available in the browser build: chat",
        "close" | "quit" | "exit" if clean.iter().any(|arg| arg == "--all") => {
            "Command is not available in the browser build: close --all"
        }
        _ => return None,
    };
    Some(message.to_string())
}

fn full_help_text() -> String {
    let source = include_str!("../../../cli/src/output.rs");
    let Some(start) = source.find("pub fn print_help()") else {
        return "agent-browser - fast browser automation CLI for AI agents".to_string();
    };
    extract_raw_string(&source[start..])
        .map(decode_println_format)
        .unwrap_or_else(|| "agent-browser - fast browser automation CLI for AI agents".to_string())
}

fn command_help_text(command: &str) -> Option<String> {
    let source = include_str!("../../../cli/src/output.rs");
    let start = source.find("pub fn print_command_help")?;
    let body = &source[start..];
    let needle = format!("\"{}\"", command);
    let command_index = body.find(&needle)?;
    extract_raw_string(&body[command_index..]).map(|text| decode_println_format(text.trim()))
}

fn extract_raw_string(source: &str) -> Option<String> {
    let bytes = source.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'r' {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        let mut hashes = 0;
        while bytes.get(cursor) == Some(&b'#') {
            hashes += 1;
            cursor += 1;
        }

        if bytes.get(cursor) != Some(&b'"') {
            index += 1;
            continue;
        }

        let content_start = cursor + 1;
        let end_marker = format!("\"{}", "#".repeat(hashes));
        let content = &source[content_start..];
        let end = content.find(&end_marker)?;
        return Some(content[..end].to_string());
    }

    None
}

fn decode_println_format(text: impl AsRef<str>) -> String {
    text.as_ref().replace("{{", "{").replace("}}", "}")
}

fn wasm_flags(args: &[String]) -> Flags {
    let mut parsed = default_flags();
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--json" => {
                let (value, consumed) = parse_bool_arg(args, index);
                parsed.json = value;
                if consumed {
                    index += 1;
                }
            }
            "--content-boundaries" => {
                let (value, consumed) = parse_bool_arg(args, index);
                parsed.content_boundaries = value;
                if consumed {
                    index += 1;
                }
            }
            "--max-output" => {
                if let Some(value) = args.get(index + 1).and_then(|value| value.parse().ok()) {
                    parsed.max_output = Some(value);
                    index += 1;
                }
            }
            "--cdp" => {
                if let Some(value) = args.get(index + 1) {
                    parsed.cdp = Some(value.clone());
                    index += 1;
                }
            }
            "--session" => {
                if let Some(value) = args.get(index + 1) {
                    parsed.session = value.clone();
                    index += 1;
                }
            }
            "--input-mode" => {
                if let Some(value) = args.get(index + 1) {
                    parsed.input_mode = value.clone();
                    index += 1;
                }
            }
            "-v" | "--verbose" => parsed.verbose = true,
            "-q" | "--quiet" => parsed.quiet = true,
            _ => {}
        }
        index += 1;
    }

    parsed
}

include!(concat!(env!("OUT_DIR"), "/default_flags.rs"));

fn parse_bool_arg(args: &[String], index: usize) -> (bool, bool) {
    match args.get(index + 1).map(|value| value.as_str()) {
        Some("true") => (true, true),
        Some("false") => (false, true),
        _ => (true, false),
    }
}

async fn execute(command: Value, transport: Function) -> Response {
    let id = command
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();

    let mut client = WasmCdpClient::new(transport);
    if let Err(error) = client.connect().await {
        return response_error(error);
    }

    match execute_action(&command, &mut client).await {
        Ok(data) => Response {
            success: true,
            data: Some(data),
            error: None,
            warning: None,
        },
        Err(error) => Response {
            success: false,
            data: None,
            error: Some(if id.is_empty() {
                error
            } else {
                error.to_string()
            }),
            warning: None,
        },
    }
}

fn response_error(error: String) -> Response {
    Response {
        success: false,
        data: None,
        error: Some(error),
        warning: None,
    }
}

struct WasmCdpClient {
    transport: Function,
    session_id: Option<String>,
}

impl WasmCdpClient {
    fn new(transport: Function) -> Self {
        Self {
            transport,
            session_id: None,
        }
    }

    fn mouse_state(&self) -> MouseState {
        MOUSE_STATES.with(|states| {
            states
                .get(&self.transport)
                .as_string()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default()
        })
    }

    async fn mouse_event(&self, params: Value) -> Result<(), String> {
        self.send_session("Input.dispatchMouseEvent", params.clone())
            .await?;
        let mut state = self.mouse_state();
        state.x = params["x"].as_f64().unwrap_or(state.x);
        state.y = params["y"].as_f64().unwrap_or(state.y);
        state.buttons = params["buttons"].as_u64().unwrap_or(state.buttons as u64) as u32;
        MOUSE_STATES.with(|states| {
            states.set(
                &self.transport,
                &JsValue::from_str(&serde_json::to_string(&state).unwrap()),
            );
        });
        Ok(())
    }

    async fn connect(&mut self) -> Result<(), String> {
        let targets = self.send_browser("Target.getTargets", json!({})).await?;
        let target_id = targets
            .get("targetInfos")
            .and_then(|value| value.as_array())
            .and_then(|targets| {
                targets
                    .iter()
                    .find(|target| {
                        target
                            .get("type")
                            .and_then(|value| value.as_str())
                            .is_some_and(|kind| kind == "page" || kind == "iframe")
                    })
                    .or_else(|| targets.first())
            })
            .and_then(|target| target.get("targetId"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| "CDP endpoint did not expose a target".to_string())?
            .to_string();

        let attached = self
            .send_browser(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        let session_id = attached
            .get("sessionId")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "CDP endpoint did not return a sessionId".to_string())?
            .to_string();
        self.session_id = Some(session_id.clone());

        for method in ["Runtime.enable", "DOM.enable", "Accessibility.enable"] {
            let _ = self.send_session(method, json!({})).await;
        }

        Ok(())
    }

    async fn send_browser(&self, method: &str, params: Value) -> Result<Value, String> {
        self.send(method, params, None).await
    }

    async fn send_session(&self, method: &str, params: Value) -> Result<Value, String> {
        let session_id = self
            .session_id
            .as_deref()
            .ok_or_else(|| "CDP session is not attached".to_string())?;
        self.send(method, params, Some(session_id)).await
    }

    async fn send(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let params = serde_json::to_string(&params)
            .map_err(|error| format!("Failed to serialize CDP params: {}", error))?;
        let result = self
            .transport
            .call3(
                &JsValue::NULL,
                &JsValue::from_str(method),
                &JsValue::from_str(&params),
                &JsValue::from_str(session_id.unwrap_or("")),
            )
            .map_err(js_error)?;
        let promise = result
            .dyn_into::<Promise>()
            .map_err(|_| "CDP transport did not return a Promise".to_string())?;
        let value = JsFuture::from(promise).await.map_err(js_error)?;
        let text = value
            .as_string()
            .ok_or_else(|| "CDP transport did not return JSON text".to_string())?;
        serde_json::from_str(&text)
            .map_err(|error| format!("Invalid CDP transport response: {}", error))
    }
}

fn js_error(value: JsValue) -> String {
    if let Some(text) = value.as_string() {
        return text;
    }
    js_sys::Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .unwrap_or_else(|| "JavaScript transport error".to_string())
}

async fn execute_action(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    match action(command) {
        "launch" => Ok(json!({})),
        "navigate" => navigate(command, client).await,
        "url" => url(client).await,
        "cdp_url" => Ok(json!({ "cdpUrl": "in-page://icdp" })),
        "title" => title(client).await,
        "content" => text_content(client).await,
        "evaluate" => evaluate_command(command, client).await,
        "snapshot" => snapshot(command, client).await,
        "click" => click(command, client, 1).await,
        "dblclick" => click(command, client, 2).await,
        "fill" => fill(command, client).await,
        "type" => type_text(command, client).await,
        "press" => press(command, client).await,
        "keyboard" => keyboard(command, client).await,
        "hover" => hover(command, client).await,
        "mousemove" => mouse_move(command, client).await,
        "mousedown" => mouse_button(command, client, true).await,
        "mouseup" => mouse_button(command, client, false).await,
        "wheel" => mouse_wheel(command, client).await,
        "drag" => drag(command, client).await,
        "focus" => focus(command, client).await,
        "scroll" => scroll(command, client).await,
        "scrollintoview" => scroll_into_view(command, client).await,
        "select" => select(command, client).await,
        "check" => check(command, client, true).await,
        "uncheck" => check(command, client, false).await,
        "wait" => wait(command, client).await,
        "waitforurl" => wait_for_url(command, client).await,
        "waitforloadstate" => Ok(json!({})),
        "waitforfunction" => wait_for_function(command, client).await,
        "gettext" => get_text(command, client).await,
        "innerhtml" => get_html(command, client).await,
        "inputvalue" => get_value(command, client).await,
        "getattribute" => get_attribute(command, client).await,
        "count" => count(command, client).await,
        "boundingbox" => bounding_box(command, client).await,
        "styles" => styles(command, client).await,
        "isvisible" => state_check(command, client, "visible").await,
        "isenabled" => state_check(command, client, "enabled").await,
        "ischecked" => state_check(command, client, "checked").await,
        other => Err(format!(
            "Command is not available in the browser build: {}",
            other
        )),
    }
}

fn action(command: &Value) -> &str {
    command
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or("")
}

fn required_string<'a>(command: &'a Value, key: &str) -> Result<&'a str, String> {
    command
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} is required", key))
}

async fn navigate(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let url = required_string(command, "url")?;
    client
        .send_session("Page.navigate", json!({ "url": url }))
        .await?;
    let title = evaluate_value(client, "document.title").await?;
    let current_url = evaluate_value(client, "location.href").await?;
    Ok(json!({
        "url": current_url.as_str().unwrap_or(url),
        "title": title.as_str().unwrap_or(""),
    }))
}

async fn url(client: &mut WasmCdpClient) -> Result<Value, String> {
    let url = evaluate_value(client, "location.href").await?;
    Ok(json!({ "url": url.as_str().unwrap_or("") }))
}

async fn title(client: &mut WasmCdpClient) -> Result<Value, String> {
    let title = evaluate_value(client, "document.title").await?;
    Ok(json!({ "title": title.as_str().unwrap_or("") }))
}

async fn text_content(client: &mut WasmCdpClient) -> Result<Value, String> {
    let text = evaluate_value(client, "document.body ? document.body.innerText : ''").await?;
    let origin = evaluate_value(client, "location.href").await?;
    Ok(json!({
        "text": text.as_str().unwrap_or(""),
        "origin": origin.as_str().unwrap_or(""),
    }))
}

async fn evaluate_command(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let script = required_string(command, "script")?;
    let result = evaluate_value(client, script).await?;
    let origin = evaluate_value(client, "location.href").await?;
    Ok(json!({ "result": result, "origin": origin.as_str().unwrap_or("") }))
}

async fn snapshot(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    client.send_session("DOM.enable", json!({})).await?;
    client
        .send_session("Accessibility.enable", json!({}))
        .await?;
    let result = client
        .send_session("Accessibility.getFullAXTree", json!({}))
        .await?;
    let nodes_value = result
        .get("nodes")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let nodes: Vec<snapshot_format::AxNode> = serde_json::from_value(nodes_value)
        .map_err(|error| format!("Invalid Accessibility.getFullAXTree response: {}", error))?;
    let options = snapshot_format::SnapshotOptions {
        interactive: command
            .get("interactive")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        compact: command
            .get("compact")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        depth: command
            .get("maxDepth")
            .and_then(|value| value.as_u64())
            .map(|value| value as usize),
        urls: command
            .get("urls")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    };
    let formatted = snapshot_format::format_snapshot(&nodes, &options)?;

    STATE.with(|state| {
        state.borrow_mut().refs = formatted.refs.clone();
    });

    let refs = formatted
        .refs
        .iter()
        .map(|(ref_id, entry)| {
            (
                ref_id.clone(),
                json!({ "role": entry.role, "name": entry.name }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let origin = evaluate_value(client, "location.href").await?;

    Ok(json!({
        "snapshot": formatted.output,
        "origin": origin.as_str().unwrap_or(""),
        "refs": refs,
    }))
}

async fn element_center(client: &mut WasmCdpClient, selector: &str) -> Result<(f64, f64), String> {
    let point = element_value(
        client,
        selector,
        r#"(element) => {
            element.scrollIntoView({ block: "center", inline: "center" });
            const box = element.getBoundingClientRect();
            return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
        }"#,
    )
    .await?;
    let x = point
        .get("x")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    let y = point
        .get("y")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    Ok((x, y))
}

async fn move_mouse(client: &WasmCdpClient, x: f64, y: f64, command: &Value) -> Result<(), String> {
    let start = client.mouse_state();
    let distance = (x - start.x).hypot(y - start.y);
    let mode = command["inputMode"].as_str().unwrap_or("instant");
    let human = mode == "human";
    let mut duration = command["duration"].as_u64().unwrap_or(0);
    if human && duration == 0 {
        duration = (80.0 + distance * 0.35).clamp(100.0, 700.0) as u64;
    }
    let requested_steps = command["steps"]
        .as_u64()
        .map(|steps| steps.min(240) as usize)
        .or_else(|| (mode == "instant" && duration == 0).then_some(1));
    let steps = crate::motion::interpolated_mouse_steps(distance, duration, requested_steps, human);
    let seed = command["seed"].as_u64().unwrap_or(0);
    let bend = if human && distance > 0.0 {
        let mixed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let unit = ((mixed >> 11) as f64) / ((1_u64 << 53) as f64);
        (unit * 2.0 - 1.0) * (distance * 0.08).min(36.0)
    } else {
        0.0
    };
    let (perp_x, perp_y) = if distance > 0.0 {
        (-(y - start.y) / distance, (x - start.x) / distance)
    } else {
        (0.0, 0.0)
    };
    let started = monotonic_now();
    for i in 1..=steps {
        let (x, y) = crate::motion::interpolated_mouse_point(
            start.x, start.y, x, y, perp_x, perp_y, bend, i, steps,
        );
        client
            .mouse_event(json!({ "type": "mouseMoved", "x": x, "y": y, "buttons": start.buttons }))
            .await?;
        // One monotonic schedule includes transport latency, like the native client.
        let remaining = started + duration as f64 * i as f64 / steps as f64 - monotonic_now();
        if remaining > 0.0 {
            JsFuture::from(Promise::new(&mut |resolve, _| {
                set_timeout(&resolve, remaining)
            }))
            .await
            .map_err(js_error)?;
        }
    }
    Ok(())
}

async fn mouse_move(command: &Value, client: &WasmCdpClient) -> Result<Value, String> {
    let x = command["x"].as_f64().unwrap_or(0.0);
    let y = command["y"].as_f64().unwrap_or(0.0);
    move_mouse(client, x, y, command).await?;
    Ok(json!({ "moved": true, "x": x, "y": y }))
}

async fn mouse_button(
    command: &Value,
    client: &WasmCdpClient,
    down: bool,
) -> Result<Value, String> {
    let state = client.mouse_state();
    let button = command["button"].as_str().unwrap_or("left");
    let mask = match button {
        "left" => 1,
        "right" => 2,
        "middle" => 4,
        "back" => 8,
        "forward" => 16,
        _ => return Err(format!("Invalid mouse button: {button}")),
    };
    let buttons = if down {
        state.buttons | mask
    } else {
        state.buttons & !mask
    };
    client
        .mouse_event(json!({
            "type": if down { "mousePressed" } else { "mouseReleased" },
            "x": state.x, "y": state.y, "button": button, "buttons": buttons,
            "clickCount": command["clickCount"].as_i64().unwrap_or(1),
        }))
        .await?;
    Ok(json!({ "button": button }))
}

async fn mouse_wheel(command: &Value, client: &WasmCdpClient) -> Result<Value, String> {
    let state = client.mouse_state();
    client
        .mouse_event(json!({
            "type": "mouseWheel", "x": state.x, "y": state.y, "buttons": state.buttons,
            "deltaX": command["deltaX"].as_f64().unwrap_or(0.0),
            "deltaY": command["deltaY"].as_f64().unwrap_or(0.0),
        }))
        .await?;
    Ok(json!({ "scrolled": true }))
}

async fn drag(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let (sx, sy) = element_center(client, required_string(command, "source")?).await?;
    let (tx, ty) = element_center(client, required_string(command, "target")?).await?;
    let human = command["inputMode"].as_str() == Some("human");
    let mut movement = command.clone();
    if !human {
        movement["steps"] = json!(1);
    }
    move_mouse(client, sx, sy, &movement).await?;
    mouse_button(&json!({ "button": "left" }), client, true).await?;
    movement["duration"] = json!(if human { 250 } else { 100 });
    if !human {
        movement["steps"] = json!(10);
    }
    movement["seed"] = json!(command["seed"].as_u64().unwrap_or(0).wrapping_add(1));
    let moved = move_mouse(client, tx, ty, &movement).await;
    // Release even on a failed move so the next command does not inherit a held button.
    let released = mouse_button(&json!({ "button": "left" }), client, false).await;
    moved?;
    released?;
    Ok(json!({ "dragged": true }))
}

async fn click(
    command: &Value,
    client: &mut WasmCdpClient,
    click_count: i64,
) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let (x, y) = element_center(client, selector).await?;
    let mut movement = command.clone();
    if command["inputMode"].as_str() == Some("smooth") {
        movement["duration"] = json!(200);
    }
    move_mouse(client, x, y, &movement).await?;
    for count in 1..=click_count {
        let button =
            json!({ "button": command["button"].as_str().unwrap_or("left"), "clickCount": count });
        mouse_button(&button, client, true).await?;
        mouse_button(&button, client, false).await?;
    }
    Ok(json!({}))
}

async fn fill(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let text = required_string(command, "value")?;
    focus_selector(client, selector).await?;
    element_value(
        client,
        selector,
        r#"(element) => {
            if ("value" in element) {
                element.value = "";
                element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContentBackward" }));
                element.dispatchEvent(new Event("change", { bubbles: true }));
            } else {
                element.textContent = "";
            }
            return true;
        }"#,
    )
    .await?;
    client
        .send_session("Input.insertText", json!({ "text": text }))
        .await?;
    Ok(json!({}))
}

async fn type_text(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let text = required_string(command, "text")?;
    if command
        .get("clear")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return fill(command, client).await;
    }
    if let Some(selector) = command.get("selector").and_then(|value| value.as_str()) {
        focus_selector(client, selector).await?;
    }
    client
        .send_session("Input.insertText", json!({ "text": text }))
        .await?;
    Ok(json!({}))
}

async fn press(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let key = required_string(command, "key")?;
    let text = if key.chars().count() == 1 { key } else { "" };
    client
        .send_session(
            "Input.dispatchKeyEvent",
            json!({ "type": "keyDown", "key": key, "text": text }),
        )
        .await?;
    client
        .send_session(
            "Input.dispatchKeyEvent",
            json!({ "type": "keyUp", "key": key }),
        )
        .await?;
    Ok(json!({}))
}

async fn keyboard(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let text = required_string(command, "text")?;
    match command.get("subaction").and_then(|value| value.as_str()) {
        Some("type") | Some("insertText") => {
            client
                .send_session("Input.insertText", json!({ "text": text }))
                .await?;
            Ok(json!({}))
        }
        _ => Err("Unsupported keyboard subcommand".to_string()),
    }
}

async fn hover(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let (x, y) = element_center(client, selector).await?;
    move_mouse(client, x, y, command).await?;
    Ok(json!({}))
}

async fn focus(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    focus_selector(client, selector).await?;
    Ok(json!({}))
}

async fn scroll(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let direction = command
        .get("direction")
        .and_then(|value| value.as_str())
        .unwrap_or("down");
    let amount = command
        .get("amount")
        .or_else(|| command.get("distance"))
        .and_then(|value| value.as_f64())
        .unwrap_or(500.0);
    let (delta_x, delta_y) = match direction {
        "up" => (0.0, -amount),
        "down" => (0.0, amount),
        "left" => (-amount, 0.0),
        "right" => (amount, 0.0),
        _ => (0.0, amount),
    };
    evaluate_value(
        client,
        &format!("window.scrollBy({}, {})", delta_x, delta_y),
    )
    .await?;
    Ok(json!({}))
}

async fn scroll_into_view(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    element_value(
        client,
        selector,
        r#"(element) => {
            element.scrollIntoView({ block: "center", inline: "center" });
            return true;
        }"#,
    )
    .await?;
    Ok(json!({}))
}

async fn select(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let values = command.get("values").cloned().unwrap_or(Value::Null);
    let values = if let Some(value) = values.as_str() {
        json!([value])
    } else {
        values
    };
    let values_json = serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string());
    element_value(
        client,
        selector,
        &format!(
            r#"(element) => {{
                if (!(element instanceof HTMLSelectElement)) throw new Error("Element is not a select");
                const values = new Set({});
                for (const option of element.options) option.selected = values.has(option.value) || values.has(option.text);
                element.dispatchEvent(new Event("input", {{ bubbles: true }}));
                element.dispatchEvent(new Event("change", {{ bubbles: true }}));
                return true;
            }}"#,
            values_json
        ),
    )
    .await?;
    Ok(json!({}))
}

async fn check(
    command: &Value,
    client: &mut WasmCdpClient,
    checked: bool,
) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    element_value(
        client,
        selector,
        &format!(
            r#"(element) => {{
                if (!("checked" in element)) throw new Error("Element is not checkable");
                element.checked = {};
                element.dispatchEvent(new Event("input", {{ bubbles: true }}));
                element.dispatchEvent(new Event("change", {{ bubbles: true }}));
                return true;
            }}"#,
            checked
        ),
    )
    .await?;
    Ok(json!({}))
}

async fn wait(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    if let Some(timeout) = command.get("timeout").and_then(|value| value.as_u64()) {
        evaluate_value(
            client,
            &format!("new Promise(resolve => setTimeout(resolve, {}))", timeout),
        )
        .await?;
        return Ok(json!({}));
    }

    if let Some(text) = command.get("text").and_then(|value| value.as_str()) {
        let text = serde_json::to_string(text).unwrap_or_default();
        evaluate_value(
            client,
            &format!(
                r#"new Promise((resolve, reject) => {{
                    const started = Date.now();
                    const tick = () => {{
                        if ((document.body?.innerText || "").includes({text})) return resolve(true);
                        if (Date.now() - started > 5000) return reject(new Error("Timed out waiting for text"));
                        setTimeout(tick, 100);
                    }};
                    tick();
                }})"#
            ),
        )
        .await?;
        return Ok(json!({}));
    }

    let selector = required_string(command, "selector")?;
    let selector = serde_json::to_string(selector).unwrap_or_default();
    evaluate_value(
        client,
        &format!(
            r#"new Promise((resolve, reject) => {{
                const started = Date.now();
                const tick = () => {{
                    if (document.querySelector({selector})) return resolve(true);
                    if (Date.now() - started > 5000) return reject(new Error("Timed out waiting for selector"));
                    setTimeout(tick, 100);
                }};
                tick();
            }})"#
        ),
    )
    .await?;
    Ok(json!({}))
}

async fn wait_for_url(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let pattern = required_string(command, "url")?;
    let pattern = serde_json::to_string(pattern).unwrap_or_default();
    evaluate_value(
        client,
        &format!(
            r#"new Promise((resolve, reject) => {{
                const expected = {pattern}.replace(/\*\*/g, "");
                const started = Date.now();
                const tick = () => {{
                    if (location.href.includes(expected)) return resolve(true);
                    if (Date.now() - started > 5000) return reject(new Error("Timed out waiting for URL"));
                    setTimeout(tick, 100);
                }};
                tick();
            }})"#
        ),
    )
    .await?;
    Ok(json!({}))
}

async fn wait_for_function(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let expression = required_string(command, "expression")?;
    evaluate_value(
        client,
        &format!(
            r#"new Promise((resolve, reject) => {{
                const started = Date.now();
                const tick = () => {{
                    if (({})) return resolve(true);
                    if (Date.now() - started > 5000) return reject(new Error("Timed out waiting for function"));
                    setTimeout(tick, 100);
                }};
                tick();
            }})"#,
            expression
        ),
    )
    .await?;
    Ok(json!({}))
}

async fn get_text(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let text = element_value(
        client,
        selector,
        r#"(element) => element.innerText || element.textContent || """#,
    )
    .await?;
    let origin = evaluate_value(client, "location.href").await?;
    Ok(json!({ "text": text.as_str().unwrap_or(""), "origin": origin.as_str().unwrap_or("") }))
}

async fn get_html(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let html = element_value(client, selector, r#"(element) => element.innerHTML"#).await?;
    let origin = evaluate_value(client, "location.href").await?;
    Ok(json!({ "html": html.as_str().unwrap_or(""), "origin": origin.as_str().unwrap_or("") }))
}

async fn get_value(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let value = element_value(
        client,
        selector,
        r#"(element) => "value" in element ? element.value : element.textContent || "" "#,
    )
    .await?;
    Ok(json!({ "value": value.as_str().unwrap_or("") }))
}

async fn get_attribute(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let attribute = required_string(command, "attribute")?;
    let expression = format!(
        r#"(element) => element.getAttribute({})"#,
        serde_json::to_string(attribute).unwrap_or_default()
    );
    let value = element_value(client, selector, &expression).await?;
    Ok(json!({ "value": value.as_str().unwrap_or("") }))
}

async fn count(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let value = evaluate_value(
        client,
        &format!(
            "document.querySelectorAll({}).length",
            serde_json::to_string(selector).unwrap_or_default()
        ),
    )
    .await?;
    Ok(json!({ "count": value.as_i64().unwrap_or(0) }))
}

async fn bounding_box(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    element_value(
        client,
        selector,
        r#"(element) => {
            const rect = element.getBoundingClientRect();
            return {
                x: Math.round(rect.x),
                y: Math.round(rect.y),
                width: Math.round(rect.width),
                height: Math.round(rect.height),
            };
        }"#,
    )
    .await
}

async fn styles(command: &Value, client: &mut WasmCdpClient) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let styles = element_value(
        client,
        selector,
        r#"(element) => {
            const computed = getComputedStyle(element);
            return {
                display: computed.display,
                visibility: computed.visibility,
                color: computed.color,
                backgroundColor: computed.backgroundColor,
                fontSize: computed.fontSize,
                lineHeight: computed.lineHeight,
            };
        }"#,
    )
    .await?;
    Ok(json!({ "styles": styles }))
}

async fn state_check(
    command: &Value,
    client: &mut WasmCdpClient,
    kind: &str,
) -> Result<Value, String> {
    let selector = required_string(command, "selector")?;
    let expression = match kind {
        "visible" => {
            r#"(element) => {
                const rect = element.getBoundingClientRect();
                const style = getComputedStyle(element);
                return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none";
            }"#
        }
        "enabled" => {
            r#"(element) => !element.disabled && element.getAttribute("aria-disabled") !== "true""#
        }
        "checked" => {
            r#"(element) => Boolean(element.checked || element.getAttribute("aria-checked") === "true")"#
        }
        _ => return Err(format!("Unsupported state check: {}", kind)),
    };
    let value = element_value(client, selector, expression).await?;
    Ok(json!({ kind: value.as_bool().unwrap_or(false) }))
}

async fn focus_selector(client: &mut WasmCdpClient, selector: &str) -> Result<(), String> {
    element_value(
        client,
        selector,
        r#"(element) => {
            if (typeof element.focus === "function") element.focus();
            return true;
        }"#,
    )
    .await?;
    Ok(())
}

async fn element_value(
    client: &mut WasmCdpClient,
    selector: &str,
    function_source: &str,
) -> Result<Value, String> {
    if let Some(ref_id) = normalize_ref(selector) {
        let entry = STATE.with(|state| state.borrow().refs.get(&ref_id).cloned());
        let entry = entry.ok_or_else(|| {
            format!(
                "Ref not found: @{}. Run agent-browser snapshot again.",
                ref_id
            )
        })?;
        let resolved = client
            .send_session(
                "DOM.resolveNode",
                json!({ "backendNodeId": entry.backend_node_id }),
            )
            .await?;
        let object_id = resolved
            .get("object")
            .and_then(|object| object.get("objectId"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                format!(
                    "Ref is stale: @{}. Run agent-browser snapshot again.",
                    ref_id
                )
            })?;
        let result = client
            .send_session(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": format!("function() {{ return ({function_source})(this); }}"),
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
            )
            .await?;
        return runtime_result_value(&result);
    }

    let selector = serde_json::to_string(selector).unwrap_or_default();
    evaluate_value(
        client,
        &format!(
            r#"(() => {{
                const selector = {selector};
                const element = document.querySelector(selector);
                if (!element) throw new Error("No element matches selector: " + selector);
                return ({function_source})(element);
            }})()"#
        ),
    )
    .await
}

fn normalize_ref(selector: &str) -> Option<String> {
    let value = selector.strip_prefix('@').unwrap_or(selector);
    if value.len() > 1
        && value.starts_with('e')
        && value[1..].chars().all(|char| char.is_ascii_digit())
    {
        Some(value.to_string())
    } else {
        None
    }
}

async fn evaluate_value(client: &mut WasmCdpClient, expression: &str) -> Result<Value, String> {
    let response = client
        .send_session(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )
        .await?;
    runtime_result_value(&response)
}

fn runtime_result_value(response: &Value) -> Result<Value, String> {
    if let Some(details) = response.get("exceptionDetails") {
        let text = details
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or("JavaScript exception");
        let description = details
            .get("exception")
            .and_then(|value| value.get("description"))
            .and_then(|value| value.as_str())
            .unwrap_or(text);
        return Err(description.to_string());
    }

    let remote = response.get("result").unwrap_or(response);
    if let Some(value) = remote.get("value") {
        return Ok(value.clone());
    }
    if let Some(value) = remote
        .get("unserializableValue")
        .and_then(|value| value.as_str())
    {
        return Ok(Value::String(value.to_string()));
    }
    if let Some(value) = remote.get("description").and_then(|value| value.as_str()) {
        return Ok(Value::String(value.to_string()));
    }
    Ok(Value::Null)
}

fn format_response(response: &Response, action: Option<&str>, flags: &Flags) -> RunOutput {
    let options = OutputOptions::from_flags(flags);
    if options.json {
        return RunOutput::success(serde_json::to_string(response).unwrap_or_default());
    }

    if !response.success {
        return RunOutput::error(format!(
            "{} {}",
            crate::color::error_indicator(),
            response.error.as_deref().unwrap_or("Unknown error")
        ));
    }

    let Some(data) = &response.data else {
        return RunOutput::success("");
    };

    if let Some(url) = data.get("url").and_then(|value| value.as_str()) {
        if let Some(title) = data.get("title").and_then(|value| value.as_str()) {
            return RunOutput::success(format!(
                "{} {}\n  {}",
                crate::color::success_indicator(),
                crate::color::bold(title),
                crate::color::dim(url)
            ));
        }
        return RunOutput::success(url);
    }

    if let Some(cdp_url) = data.get("cdpUrl").and_then(|value| value.as_str()) {
        return RunOutput::success(cdp_url);
    }

    if let Some(snapshot) = data.get("snapshot").and_then(|value| value.as_str()) {
        let origin = data.get("origin").and_then(|value| value.as_str());
        return RunOutput::success(page_content(snapshot, origin, &options));
    }

    if let Some(title) = data.get("title").and_then(|value| value.as_str()) {
        return RunOutput::success(title);
    }

    if let Some(text) = data.get("text").and_then(|value| value.as_str()) {
        let origin = data.get("origin").and_then(|value| value.as_str());
        return RunOutput::success(page_content(text, origin, &options));
    }

    if let Some(html) = data.get("html").and_then(|value| value.as_str()) {
        let origin = data.get("origin").and_then(|value| value.as_str());
        return RunOutput::success(page_content(html, origin, &options));
    }

    if let Some(value) = data.get("value").and_then(|value| value.as_str()) {
        return RunOutput::success(value);
    }

    if let Some(count) = data.get("count").and_then(|value| value.as_i64()) {
        return RunOutput::success(count.to_string());
    }

    if action == Some("boundingbox") {
        return RunOutput::success(format!(
            "x:      {}\ny:      {}\nwidth:  {}\nheight: {}",
            number(data.get("x")),
            number(data.get("y")),
            number(data.get("width")),
            number(data.get("height"))
        ));
    }

    if let Some(styles) = data.get("styles").and_then(|value| value.as_object()) {
        let mut lines = Vec::new();
        for (key, value) in styles {
            let value = value
                .as_str()
                .map(ToString::to_string)
                .unwrap_or_else(|| value.to_string());
            lines.push(format!("{}: {}", key, value));
        }
        return RunOutput::success(lines.join("\n"));
    }

    for key in ["visible", "enabled", "checked"] {
        if let Some(value) = data.get(key).and_then(|value| value.as_bool()) {
            return RunOutput::success(value.to_string());
        }
    }

    if let Some(result) = data.get("result") {
        let origin = data.get("origin").and_then(|value| value.as_str());
        return RunOutput::success(page_content(
            &serde_json::to_string_pretty(result).unwrap_or_default(),
            origin,
            &options,
        ));
    }

    if data.as_object().is_some_and(|object| object.is_empty()) {
        return RunOutput::success("");
    }

    RunOutput::success(serde_json::to_string_pretty(data).unwrap_or_default())
}

fn page_content(content: &str, origin: Option<&str>, options: &OutputOptions) -> String {
    let content = truncate_if_needed(content, options.max_output);
    if !options.content_boundaries {
        return content;
    }

    let nonce = "wasm";
    format!(
        "--- AGENT_BROWSER_PAGE_CONTENT nonce={} origin={} ---\n{}\n--- END_AGENT_BROWSER_PAGE_CONTENT nonce={} ---",
        nonce,
        origin.unwrap_or("unknown"),
        content,
        nonce
    )
}

fn truncate_if_needed(content: &str, limit: Option<usize>) -> String {
    let Some(limit) = limit else {
        return content.to_string();
    };
    if content.chars().count() <= limit {
        return content.to_string();
    }
    let truncated = content.chars().take(limit).collect::<String>();
    let total = content.chars().count();
    format!(
        "{}\n[truncated: showing {} of {} chars. Use --max-output to adjust]",
        truncated, limit, total
    )
}

fn number(value: Option<&Value>) -> String {
    value
        .and_then(|value| value.as_f64())
        .map(|value| {
            if value.fract() == 0.0 {
                format!("{}", value as i64)
            } else {
                value.to_string()
            }
        })
        .unwrap_or_else(|| "0".to_string())
}
