use knowledge_builder::build_package;
use serde_json::Value;
use std::{env, fs, path::Path, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 3 || args[0] != "build" {
        eprintln!("usage: knowledge-builder build <profile.yml> <output-package-directory>");
        return ExitCode::from(2);
    }
    match build_package(Path::new(&args[1]), Path::new(&args[2])) {
        Ok(()) => {
            let manifest_path = Path::new(&args[2]).join("manifest.json");
            if let Ok(contents) = fs::read_to_string(manifest_path) {
                if let Ok(manifest) = serde_json::from_str::<Value>(&contents) {
                    if let Some(collection) = manifest.get("collection") {
                        let outcomes = collection["sources"].as_array().cloned().unwrap_or_default();
                        let count = |statuses: &[&str]| outcomes.iter().filter(|outcome|
                            outcome["status"].as_str().is_some_and(|status| statuses.contains(&status))
                        ).count();
                        println!(
                            "collection: {} ({} succeeded, {} partial, {} unavailable/failed, {} skipped/unconfigured)",
                            collection["status"].as_str().unwrap_or("unknown"),
                            count(&["succeeded"]),
                            count(&["partial", "succeeded-with-truncation"]),
                            count(&["unavailable", "permission-denied", "failed"]),
                            count(&["skipped-by-profile", "not-configured"])
                        );
                    }
                }
            }
            println!("package created: {}", args[2]);
            ExitCode::SUCCESS
        }
        Err(error) => { eprintln!("build failed: {error}"); ExitCode::FAILURE }
    }
}
