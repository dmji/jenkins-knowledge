use jenkins_gdsl_collector::collect;
use serde_json::Value;
use std::{env, fs, process};

fn main() {
    let mut args = env::args().skip(1);
    let Some(controller) = args.next() else {
        eprintln!("usage: jenkins-gdsl-collector <controller-url> <output.json>");
        process::exit(2);
    };
    let Some(output) = args.next() else {
        eprintln!("usage: jenkins-gdsl-collector <controller-url> <output.json>");
        process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("unexpected extra arguments");
        process::exit(2);
    }
    let username = env::var("JENKINS_USERNAME").ok();
    let token = env::var("JENKINS_API_TOKEN").ok();
    if username.is_some() != token.is_some() {
        eprintln!("set both JENKINS_USERNAME and JENKINS_API_TOKEN, or neither");
        process::exit(2);
    }
    let catalog: Value = match collect(&controller, username.as_deref(), token.as_deref()) {
        Ok(value) => value,
        Err(error) => { eprintln!("{error}"); process::exit(1); }
    };
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
