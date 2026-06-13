use std::env;
use std::fs;
use std::path::PathBuf;

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
}
