use std::env;
use std::fs;
use std::path::PathBuf;

// Reuse the platform-independent definitions required by the upstream parser.
// Rust items end at a closing brace with the same indentation as their declaration.
fn item(source: &str, marker: &str) -> String {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing {marker}"));
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let indent = &source[line_start..start];
    let terminator = format!("\n{indent}}}");
    let source = &source[start..];
    let end = if marker.contains("const ") {
        source.find('\n').expect("constant terminator")
    } else {
        source.find(&terminator).expect("item terminator") + terminator.len()
    };
    source[..end].to_string()
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let source_path = manifest_dir.join("../../cli/src/commands.rs");
    println!("cargo:rerun-if-changed={}", source_path.display());

    let source = fs::read_to_string(&source_path).expect("read cli/src/commands.rs");
    let native_gen_id = r#"pub fn gen_id() -> String {
    format!(
        "r{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros()
            % 1000000
    )
}"#;
    let wasm_gen_id = r#"pub fn gen_id() -> String {
    crate::wasm_support::gen_id()
}"#;

    let generated = source.replace(native_gen_id, wasm_gen_id);
    if generated == source {
        panic!("failed to patch gen_id in generated commands.rs");
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("commands.rs"), generated).expect("write generated commands.rs");
    fs::write(
        out_dir.join("default_flags.rs"),
        item(&source, "fn default_flags() -> Flags"),
    )
    .expect("write default flags");

    let read_source = |path: &str| {
        let path = manifest_dir.join("../../cli/src").join(path);
        println!("cargo:rerun-if-changed={}", path.display());
        fs::read_to_string(path).expect("read upstream source")
    };
    let plugins = read_source("plugins.rs");
    let read = read_source("read.rs");
    let webmcp = read_source("native/webmcp.rs");
    let recording = read_source("native/recording.rs");
    let actions = read_source("native/actions.rs");
    let mut support = String::from("pub mod plugins {\nuse serde::{Deserialize, Serialize};\n#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]\n#[serde(default, rename_all = \"camelCase\")]\n");
    support.push_str(&item(&plugins, "pub struct PluginConfig"));
    support.push_str("\n}\npub mod read {\n");
    for marker in [
        "const DEFAULT_TIMEOUT_MS",
        "pub enum LlmsMode",
        "pub fn parse_llms_mode",
        "pub fn default_timeout_ms",
        "pub fn parse_timeout_ms",
    ] {
        support.push_str(&item(&read, marker));
        support.push('\n');
    }
    support.push_str("}\npub mod native {\npub mod webmcp {\nuse serde_json::Value;\n");
    for marker in [
        "pub const MAX_INPUT_BYTES",
        "pub const ERR_INVALID_INPUT",
        "pub fn validate_input",
    ] {
        support.push_str(&item(&webmcp, marker));
        support.push('\n');
    }
    support.push_str("}\npub mod recording {\n");
    for marker in [
        "pub const MAX_FPS",
        "fn output_extension",
        "pub fn validate_output_path",
    ] {
        support.push_str(&item(&recording, marker));
        support.push('\n');
    }
    support.push_str("}\n}\npub mod motion {\n");
    for marker in ["fn interpolated_mouse_steps", "fn interpolated_mouse_point"] {
        support.push_str("pub ");
        support.push_str(&item(&actions, marker));
        support.push('\n');
    }
    support.push_str("}\n");
    fs::write(out_dir.join("parser_support.rs"), support).expect("write parser support");
}
