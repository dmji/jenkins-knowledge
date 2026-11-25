use jenkins_runtime::Runtime;
use serde_json::json;
use std::{env, process};

fn usage() {
    eprintln!("Usage: jenkins-runtime --package <dir|manifest.json> [--profile <name>] <search|list|get|examples|step> [arguments] [--kind <kind>] [--source <source-id>]");
    eprintln!(
        "Commands: search <query>, list, get <name-or-id>, examples <name-or-id>, step <step-name>"
    );
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1).peekable();
    let mut package = None;
    let mut profile = None;
    let mut positional = Vec::new();
    let mut kind = None;
    let mut source = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--package" | "-p" => package = args.next(),
            "--profile" => profile = args.next(),
            "--kind" => kind = args.next(),
            "--source" => source = args.next(),
            "-h" | "--help" => {
                usage();
                return Ok(());
            }
            _ => positional.push(arg),
        }
    }
    let package = package.ok_or("missing required --package")?;
    let command = positional.first().ok_or("missing command")?;
    let query = positional.get(1).map(String::as_str).unwrap_or("");
    let runtime = Runtime::open(package)?;
    if let Some(requested) = profile {
        if requested != runtime.profile() {
            return Err(format!(
                "requested profile '{requested}' does not match package profile '{}'; select the matching package explicitly",
                runtime.profile()
            )
            .into());
        }
    }
    let output = match command.as_str() {
        "search" => json!(runtime.search(query, kind.as_deref(), source.as_deref())),
        "list" => json!(runtime.search("", kind.as_deref(), source.as_deref())),
        "get" => runtime.get(query)?,
        "examples" => runtime.examples(query)?,
        "step" => runtime.pipeline_step(query)?,
        _ => {
            usage();
            return Err(format!("unknown command '{command}'").into());
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
