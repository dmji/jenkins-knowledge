use jenkins_gdsl_collector::{collect, discover};
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

    let profile = match (args.next(), args.next(), args.next()) {
        (None, None, None) => "quick".to_owned(),
        (Some(flag), Some(profile), None) if flag == "--profile" => profile,
        _ => usage(),
    };
    if profile != "quick" && profile != "gdsl" {
        eprintln!("unsupported profile '{profile}'; supported profiles: quick, gdsl");
        process::exit(2);
    }

    let username = env::var("JENKINS_USERNAME").ok();
    let token = env::var("JENKINS_API_TOKEN").ok();
    if username.is_some() != token.is_some() {
        eprintln!("set both JENKINS_USERNAME and JENKINS_API_TOKEN, or neither");
        process::exit(2);
    }

    let result = match profile.as_str() {
        "quick" => discover(&controller, username.as_deref(), token.as_deref())
            .map_err(|error| error.to_string()),
        "gdsl" => collect(&controller, username.as_deref(), token.as_deref())
            .map_err(|error| error.to_string()),
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
    let write_result = fs::write(&temporary, serialized)
        .and_then(|()| fs::rename(&temporary, &output));
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        eprintln!("could not write catalog: {error}");
        process::exit(2);
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: jenkins-gdsl-collector <controller-url> <output.json> [--profile quick|gdsl] (default: quick)"
    );
    process::exit(2);
}
