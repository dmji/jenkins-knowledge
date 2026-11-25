use jenkins_runtime::{groovy::render_groovy_stubs, Runtime};
use std::{env, fs, process};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut package = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--package" | "-p" => package = args.next(),
            "--output" | "-o" => output = args.next(),
            "-h" | "--help" => {
                println!("Usage: groovy-stubs --package <knowledge-package-dir> --output <stubs.groovy>");
                return Ok(());
            }
            _ => return Err(format!("unknown argument '{arg}'").into()),
        }
    }
    let package = package.ok_or("missing required --package")?;
    let output = output.ok_or("missing required --output")?;
    let runtime = Runtime::open(package)?;
    let stubs = render_groovy_stubs(&runtime);
    fs::write(&output, stubs)?;
    println!("Generated Groovy stubs for profile '{}' at {}", runtime.profile(), output);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}
