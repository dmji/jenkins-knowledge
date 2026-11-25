use shared_library_scanner::{scan, ScanConfig};
use serde_json::Value;
use std::{env, fs, process};
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 3 { eprintln!("usage: shared-library-scanner <config.json> <catalog.json> <diagnostics.json>"); process::exit(2); }
    let config: ScanConfig = serde_json::from_str(&read(&args[0])).unwrap_or_else(|e| { eprintln!("invalid scan configuration: {e}"); process::exit(2); });
    match scan(config) {
        Ok(output) => { write(&args[1], &output.catalog); write(&args[2], &serde_json::to_value(output.diagnostics).unwrap_or(Value::Null)); println!("catalog written to {}", args[1]); }
        Err(error) => { eprintln!("scan failed: {error}"); process::exit(1); }
    }
}
fn read(path: &str) -> String { fs::read_to_string(path).unwrap_or_else(|e| { eprintln!("cannot read {path}: {e}"); process::exit(2) }) }
fn write(path: &str, value: &Value) {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|e| { eprintln!("cannot serialize output: {e}"); process::exit(1) });
    fs::write(path, format!("{text}\n")).unwrap_or_else(|e| { eprintln!("cannot write {path}: {e}"); process::exit(1) });
}
