use jenkins_gdsl_collector::{collect, collect_script_console, discover, script_console_plan};
use serde_json::Value;
use std::{env, fs, process};

fn main() {
    let mut args = env::args().skip(1);
    let Some(controller) = args.next() else {
        usage();
    };
    let Some(output) = args.next() else {
        usage();
    };

    let mut profile = "quick".to_owned();
    let mut profile_seen = false;
    let mut dry_run = false;
    let mut confirm_script_console = false;

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--profile" if !profile_seen => {
                let Some(value) = args.next() else {
                    usage();
                };
                profile = value;
                profile_seen = true;
            }
            "--dry-run" if !dry_run => dry_run = true,
            "--confirm-script-console" if !confirm_script_console => {
                confirm_script_console = true;
            }
            _ => usage(),
        }
    }

    if profile != "quick" && profile != "gdsl" && profile != "deep" {
        eprintln!("unsupported profile '{profile}'; supported profiles: quick, gdsl, deep");
        process::exit(2);
    }
    if dry_run && (profile != "deep" || confirm_script_console) {
        eprintln!("--dry-run is only valid with --profile deep and cannot be combined with confirmation");
        process::exit(2);
    }
    if confirm_script_console && profile != "deep" {
        eprintln!("--confirm-script-console is only valid with --profile deep");
        process::exit(2);
    }
    if profile == "deep" && !dry_run && !confirm_script_console {
        eprintln!(
            "deep profile executes Jenkins Script Console; inspect --dry-run output, then pass --confirm-script-console"
        );
        process::exit(2);
    }

    let (username, token) = if dry_run {
        (None, None)
    } else {
        (
            env::var("JENKINS_USERNAME").ok(),
            env::var("JENKINS_API_TOKEN").ok(),
        )
    };
    if username.is_some() != token.is_some() {
        eprintln!("set both JENKINS_USERNAME and JENKINS_API_TOKEN, or neither");
        process::exit(2);
    }

    let result: Result<Value, String> = match profile.as_str() {
        "quick" => discover(&controller, username.as_deref(), token.as_deref())
            .map_err(|error| error.to_string()),
        "gdsl" => collect(&controller, username.as_deref(), token.as_deref())
            .map_err(|error| error.to_string()),
        "deep" if dry_run => Ok(script_console_plan()),
        "deep" => (|| -> Result<Value, String> {
            let mut catalog = discover(&controller, username.as_deref(), token.as_deref())
                .map_err(|error| error.to_string())?;
            let runtime =
                collect_script_console(&controller, username.as_deref(), token.as_deref());
            let request_count = catalog["controller"]["collection"]["requestCount"]
                .as_u64()
                .unwrap_or(0)
                + 1;
            catalog["controller"]["collection"]["profile"] = serde_json::json!("deep");
            catalog["controller"]["collection"]["requestCount"] =
                serde_json::json!(request_count);
            catalog["controller"]["runtime"] = runtime;

            let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))
                .map_err(|error| format!("could not load catalog schema: {error}"))?;
            catalog_validator::validate_catalog(&catalog, &schema)
                .map_err(|error| format!("deep discovery produced an invalid catalog: {error}"))?;
            Ok(catalog)
        })(),
        _ => unreachable!("profile was validated"),
    };
    let catalog: Value = result.unwrap_or_else(|error| {
        eprintln!("{error}");
        process::exit(1);
    });

    let serialized = match serde_json::to_vec_pretty(&catalog) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("could not serialize catalog: {error}");
            process::exit(2);
        }
    };
    let temporary = format!("{output}.tmp-{}", process::id());
    let write_result =
        fs::write(&temporary, serialized).and_then(|()| fs::rename(&temporary, &output));
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        eprintln!("could not write catalog: {error}");
        process::exit(2);
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: jenkins-gdsl-collector <controller-url> <output.json> [--profile quick|gdsl|deep] [--dry-run|--confirm-script-console] (default: quick)"
    );
    process::exit(2);
}
