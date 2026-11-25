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
    if let Err(error) = fs::write(&output, format!("{}\n", serde_json::to_string_pretty(&catalog).unwrap())) {
        eprintln!("could not write catalog: {error}");
        process::exit(2);
    }
}
