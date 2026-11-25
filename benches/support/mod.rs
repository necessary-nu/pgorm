//! Runtime metadata shared by the PostgreSQL and native benchmark reports.
// [spec:pgorm:req:turso.performance]

use crate::Result;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub(crate) fn command(root: &Path, name: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(name)
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub(crate) fn hash_file(root: &Path, path: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("hash-object")
        .arg(path)
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(format!("cannot fingerprint {}", path.display()).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(crate) fn source_fingerprint(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err("cannot enumerate source files".into());
    }
    let mut names = String::from_utf8(output.stdout)?
        .split('\0')
        .filter(|name| {
            name.ends_with(".rs") || name.ends_with("Cargo.toml") || name.ends_with("Cargo.lock")
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    names.push("Cargo.lock".to_owned());
    names.sort();
    names.dedup();
    let mut hash = Command::new("git")
        .args(["hash-object", "--stdin"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = hash
        .stdin
        .take()
        .ok_or("cannot write source fingerprint input")?;
    for name in names {
        let path = root.join(&name);
        if path.is_file() {
            input.write_all(name.as_bytes())?;
            input.write_all(&[0])?;
            input.write_all(&std::fs::read(path)?)?;
            input.write_all(&[0])?;
        }
    }
    drop(input);
    let output = hash.wait_with_output()?;
    if !output.status.success() {
        return Err("cannot fingerprint source files".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
