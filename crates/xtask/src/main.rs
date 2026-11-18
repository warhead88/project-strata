mod limits;

use std::env;
use std::error::Error;
use std::path::Path;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [task] if task == "limits" => limits::check(&root),
        [task] if task == "verify" => {
            limits::check(&root)?;
            cargo(&root, &["fmt", "--all", "--check"])?;
            cargo(
                &root,
                &[
                    "clippy",
                    "--locked",
                    "--workspace",
                    "--all-targets",
                    "--all-features",
                    "--",
                    "-D",
                    "warnings",
                ],
            )?;
            cargo(
                &root,
                &["test", "--locked", "--workspace", "--all-features"],
            )?;
            cargo(
                &root,
                &["build", "--locked", "--workspace", "--all-features"],
            )
        }
        _ => Err("usage: cargo verify | cargo limits".into()),
    }
}

fn cargo(root: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let command = format!("cargo {}", args.join(" "));
    println!("{command}");
    let status = Command::new(env!("CARGO"))
        .current_dir(root)
        .args(args)
        .status()?;
    if !status.success() {
        return Err(format!("{command} failed ({status})").into());
    }
    Ok(())
}
