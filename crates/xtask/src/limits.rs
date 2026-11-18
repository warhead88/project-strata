use std::error::Error;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy)]
struct Budget {
    lines: usize,
    bytes: u64,
}

const CODE: Budget = Budget {
    lines: 400,
    bytes: 64 * 1024,
};
const CONFIG: Budget = Budget {
    lines: 250,
    bytes: 32 * 1024,
};
const CONTENT: Budget = Budget {
    lines: 500,
    bytes: 128 * 1024,
};

pub fn check(root: &Path) -> Result<(), Box<dyn Error>> {
    let files = Command::new("git")
        .current_dir(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--deduplicate",
            "-z",
        ])
        .output()?;
    if !files.status.success() {
        return Err(format!("git ls-files: {}", String::from_utf8_lossy(&files.stderr)).into());
    }

    let mut passed = true;
    for name in str::from_utf8(&files.stdout)?.split_terminator('\0') {
        let path = Path::new(name);
        if let Some(budget) = budget(path) {
            passed &= check_file(&root.join(path), path, budget)?;
        }
    }
    if !passed {
        return Err("File size limits exceeded".into());
    }
    println!("File size limits passed");
    Ok(())
}

fn budget(path: &Path) -> Option<Budget> {
    if path.starts_with("content") {
        return Some(CONTENT);
    }

    match path.extension()?.to_str()? {
        "rs" | "wgsl" | "wesl" | "sh" | "ps1" | "py" => Some(CODE),
        "toml" | "json" | "json5" | "ron" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "xml" => {
            Some(CONFIG)
        }
        _ => None,
    }
}

fn check_file(path: &Path, relative: &Path, budget: Budget) -> Result<bool, Box<dyn Error>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(format!("{}: {error}", relative.display()).into()),
    };
    if !metadata.is_file() {
        return Ok(true);
    }

    let bytes = metadata.len();
    if bytes > budget.bytes {
        eprintln!(
            "{}: {bytes} bytes, limit {}",
            relative.display(),
            budget.bytes
        );
        return Ok(false);
    }

    let source =
        fs::read_to_string(path).map_err(|error| format!("{}: {error}", relative.display()))?;
    let lines = source.lines().count();
    if lines > budget.lines {
        eprintln!(
            "{}: {lines} lines, limit {}",
            relative.display(),
            budget.lines
        );
        return Ok(false);
    }
    Ok(true)
}
