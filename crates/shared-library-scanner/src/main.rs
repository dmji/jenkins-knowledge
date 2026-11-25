use shared_library_scanner::{scan_library, LibraryConfig};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(config_path) = args.next() else {
        eprintln!("usage: shared-library-scanner <libraries.json> [output.json]");
        return ExitCode::from(2);
    };
    let input = match fs::read_to_string(&config_path) {
        Ok(input) => input,
        Err(error) => { eprintln!("cannot read {config_path}: {error}"); return ExitCode::FAILURE; }
    };
    let configs: Vec<LibraryConfigJson> = match serde_json::from_str(&input) {
        Ok(configs) => configs,
        Err(error) => { eprintln!("invalid configuration: {error}"); return ExitCode::FAILURE; }
    };
    let mut catalogs = Vec::new();
    let mut diagnostics = Vec::new();
    for config in configs {
        let config = LibraryConfig { name: config.name, repository: config.repository, reference: config.reference };
        match scan_library(&config) {
            Ok(result) => { catalogs.push(result.catalog); diagnostics.extend(result.diagnostics); }
            Err(error) => { eprintln!("{}: {error}", config.name); return ExitCode::FAILURE; }
        }
    }
    for diagnostic in diagnostics { eprintln!("warning: {diagnostic}"); }
    let output = serde_json::to_string_pretty(&catalogs).expect("serialize catalogs");
    if let Some(path) = args.next() {
        if let Err(error) = fs::write(&path, output) { eprintln!("cannot write {path}: {error}"); return ExitCode::FAILURE; }
    } else { println!("{output}"); }
    ExitCode::SUCCESS
}

#[derive(serde::Deserialize)]
struct LibraryConfigJson { name: String, repository: String, reference: String }
