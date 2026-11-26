use knowledge_builder::{build_package_with_options, collection_plan, invalidate_cache};
use serde_json::Value;
use std::{env, fs, path::Path, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [command, profile, output] if command == "build" || command == "refresh" => {
            match build_package_with_options(
                Path::new(profile),
                Path::new(output),
                command == "refresh",
            ) {
                Ok(()) => {
                    let manifest_path = Path::new(output).join("manifest.json");
                    if let Ok(contents) = fs::read_to_string(manifest_path) {
                        if let Ok(manifest) = serde_json::from_str::<Value>(&contents) {
                            if let Some(collection) = manifest.get("collection") {
                                let outcomes = collection["sources"]
                                    .as_array()
                                    .cloned()
                                    .unwrap_or_default();
                                let count = |statuses: &[&str]| {
                                    outcomes
                                        .iter()
                                        .filter(|outcome| {
                                            outcome["status"]
                                                .as_str()
                                                .is_some_and(|status| statuses.contains(&status))
                                        })
                                        .count()
                                };
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
                    println!("package created: {}", output);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("build failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [command, profile] if command == "plan" => match collection_plan(Path::new(profile)) {
            Ok(plan) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&plan).unwrap_or_else(|_| "{}".into())
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("plan failed: {error}");
                ExitCode::FAILURE
            }
        },
        [command, profile] if command == "invalidate" => match invalidate_cache(Path::new(profile))
        {
            Ok(removed) => {
                println!("invalidated {removed} controller snapshot cache entries");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("cache invalidation failed: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage:\n  knowledge-builder plan <profile.yml>\n  knowledge-builder build <profile.yml> <output-package-directory>\n  knowledge-builder refresh <profile.yml> <output-package-directory>\n  knowledge-builder invalidate <profile.yml>");
            ExitCode::from(2)
        }
    }
}
