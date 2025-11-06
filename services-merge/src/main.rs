use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

type DescriptionMap = BTreeMap<String, Option<String>>;

fn main() {
    if let Err(err) = run() {
        eprintln!("services-merge: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            if !err.is_empty() {
                eprintln!("{err}");
            }
            eprintln!("{}", usage());
            std::process::exit(2);
        }
    };
    let template = load_map(&args.template)?;
    if template.is_empty() {
        // An empty template is technically valid, but warn to aid debugging.
        eprintln!(
            "services-merge: warning: template '{}' is empty",
            args.template.display()
        );
    }

    let mut merged = load_map(&args.target)?;
    overlay(&mut merged, template);
    write_map(&args.target, &merged)?;
    Ok(())
}

struct CliArgs {
    template: PathBuf,
    target: PathBuf,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut args = env::args().skip(1);
    let mut template = None;
    let mut target = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--template" | "-t" => {
                let value = args.next().ok_or_else(|| {
                    format!("expected path after '{arg}', found end of arguments")
                })?;
                template = Some(PathBuf::from(value));
            }
            "--target" | "-o" => {
                let value = args.next().ok_or_else(|| {
                    format!("expected path after '{arg}', found end of arguments")
                })?;
                target = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                return Err(String::new());
            }
            other => {
                return Err(format!("unexpected argument '{other}'"));
            }
        }
    }

    let template =
        template.ok_or_else(|| "missing required '--template <path>' argument".to_string())?;
    let target = target.ok_or_else(|| "missing required '--target <path>' argument".to_string())?;

    Ok(CliArgs { template, target })
}

fn usage() -> &'static str {
    "Usage: services-merge --template <template.json> --target <target.json>"
}

fn load_map(path: &Path) -> Result<DescriptionMap, Box<dyn Error>> {
    let data = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(DescriptionMap::new()),
        Err(err) => return Err(Box::new(err)),
    };

    let map: DescriptionMap = serde_json::from_str(&data)?;
    Ok(map)
}

fn overlay(target: &mut DescriptionMap, template: DescriptionMap) {
    for (key, value) in template {
        target.insert(key, value);
    }
}

fn write_map(path: &Path, map: &DescriptionMap) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(map)?;
    fs::write(path, data)?;
    Ok(())
}
