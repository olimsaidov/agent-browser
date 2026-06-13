#![allow(dead_code)]

use serde_json::json;
use wasm_bindgen::prelude::*;

#[path = "../../../cli/src/color.rs"]
mod color;

#[path = "../../../cli/src/flags.rs"]
mod flags;

#[path = "../../../cli/src/validation.rs"]
mod validation;

mod commands {
    include!(concat!(env!("OUT_DIR"), "/commands.rs"));
}

mod connection {
    use serde_json::Value;

    #[derive(serde::Serialize)]
    pub struct Response {
        pub success: bool,
        pub data: Option<Value>,
        pub error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub warning: Option<String>,
    }
}

#[path = "../../../cli/src/output.rs"]
mod output;

pub mod wasm_support {
    pub fn gen_id() -> String {
        format!("r{}", js_sys::Date::now() as u64 % 1_000_000)
    }
}

mod snapshot_format;
mod runtime;

#[wasm_bindgen]
pub async fn run_command_line(input: String, transport: js_sys::Function) -> String {
    let output = runtime::run_command_line(&input, transport).await;
    encode_output(output)
}

#[wasm_bindgen]
pub async fn run_command(input: String, transport: js_sys::Function) -> String {
    let output = runtime::run_command(&input, transport).await;
    encode_output(output)
}

#[wasm_bindgen]
pub async fn run_argv(args_json: String, transport: js_sys::Function) -> String {
    let args: Vec<String> = match serde_json::from_str(&args_json) {
        Ok(args) => args,
        Err(error) => {
            return json!({
                "stdout": "",
                "stderr": format!("Invalid argv JSON: {}", error),
                "exitCode": 1,
                "clear": false,
            })
            .to_string();
        }
    };
    let output = runtime::run_argv(args, transport).await;
    encode_output(output)
}

#[wasm_bindgen]
pub fn reset_state() {
    runtime::reset_state();
}

fn encode_output(output: runtime::RunOutput) -> String {
    serde_json::to_string(&output).unwrap_or_else(|error| {
        json!({
            "stdout": "",
            "stderr": format!("Failed to serialize WASM output: {}", error),
            "exitCode": 1,
            "clear": false,
        })
        .to_string()
    })
}
