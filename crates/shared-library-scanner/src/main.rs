use serde::Serialize;
use shared_library_scanner::{scan_library, Diagnostic, LibraryConfig};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(config_path) = args.next() else {
        eprintln!("usage: shared-library-scanner <libraries.json> [output.json] [--diagnostics <diagnostics.json>]");
        return ExitCode::from(2);
    };

    let mut catalog_output = None;
    let mut diagnostics_output = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--diagnostics" => {
                if diagnostics_output.is_some() {
                    eprintln!("--diagnostics may be specified only once");
                    return ExitCode::from(2);
                }
                let Some(path) = args.next() else {
                    eprintln!("--diagnostics requires an output path");
                    return ExitCode::from(2);
                };
                diagnostics_output = Some(path);
            }
            _ if argument.starts_with('-') => {
                eprintln!("unknown option: {argument}");
                return ExitCode::from(2);
            }
            _ if catalog_output.is_none() => catalog_output = Some(argument),
            _ => {
                eprintln!("unexpected argument: {argument}");
                return ExitCode::from(2);
            }
        }
    }

    let input = match fs::read_to_string(&config_path) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("cannot read {config_path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let configs: Vec<LibraryConfigJson> = match serde_json::from_str(&input) {
        Ok(configs) => configs,
        Err(error) => {
            eprintln!("invalid configuration: {error}");
            return ExitCode::FAILURE;
        }
    };

    let mut catalogs = Vec::new();
    let mut diagnostics = Vec::new();
    for config in configs {
        let config = LibraryConfig {
            name: config.name,
            repository: config.repository,
            reference: config.reference,
        };
        match scan_library(&config) {
            Ok(result) => {
                catalogs.push(result.catalog);
                diagnostics.extend(result.diagnostics);
            }
            Err(error) => {
                eprintln!("{}: {error}", config.name);
                return ExitCode::FAILURE;
            }
        }
    }

    for diagnostic in &diagnostics {
        eprintln!(
            "warning: {} [{}] {}: {}",
            diagnostic.code, diagnostic.source_id, diagnostic.path, diagnostic.message
        );
    }

    if let Some(path) = diagnostics_output {
        let output = serde_json::to_string_pretty(&DiagnosticsFile {
            schema_version: "1.0.0",
            diagnostics: &diagnostics,
        }).expect("serialize diagnostics");
        if let Err(error) = fs::write(&path, output) {
            eprintln!("cannot write {path}: {error}");
            return ExitCode::FAILURE;
        }
    }

    let output = serde_json::to_string_pretty(&catalogs).expect("serialize catalogs");
    if let Some(path) = catalog_output {
        if let Err(error) = fs::write(&path, output) {
            eprintln!("cannot write {path}: {error}");
            return ExitCode::FAILURE;
        }
    } else {
        println!("{output}");
    }
    ExitCode::SUCCESS
}

#[derive(serde::Deserialize)]
struct LibraryConfigJson {
    name: String,
    repository: String,
    reference: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsFile<'a> {
    schema_version: &'static str,
    diagnostics: &'a [Diagnostic],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_output_is_versioned_machine_readable_json() {
        let diagnostics = vec![Diagnostic {
            code: "dynamic-groovy-constructs".into(),
            severity: "warning".into(),
            source_id: "shared-library:platform:main@0123456789ab".into(),
            path: "vars/deploy.groovy".into(),
            message: "Dynamic Groovy constructs detected.".into(),
        }];
        let output = serde_json::to_value(DiagnosticsFile {
            schema_version: "1.0.0",
            diagnostics: &diagnostics,
        }).unwrap();

        assert_eq!(output["schemaVersion"], "1.0.0");
        assert_eq!(output["diagnostics"][0]["code"], "dynamic-groovy-constructs");
        assert_eq!(output["diagnostics"][0]["sourceId"], "shared-library:platform:main@0123456789ab");
        assert_eq!(output["diagnostics"][0]["path"], "vars/deploy.groovy");
    }
}
