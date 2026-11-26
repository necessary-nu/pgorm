//! A crate whose integration tests link as one binary sets `autotests = false`
//! and names `tests/it.rs` its one test target, so cargo compiles a suite file
//! under `tests/` only once `tests/it.rs` declares it. An undeclared file would
//! pass in silence, its tests never run, so every such crate in the checkout,
//! the detached workspaces included, is held to its declarations here.

use std::{collections::BTreeSet, path::Path, process::Command};

#[test]
fn every_suite_file_is_declared_in_its_binary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()
        .expect("git lists the checkout's files");
    assert!(output.status.success(), "git ls-files failed");
    let listed = String::from_utf8(output.stdout).expect("checkout paths are UTF-8");
    let files: Vec<&str> = listed.split('\0').filter(|path| !path.is_empty()).collect();

    let mut binaries = 0;
    let mut undeclared = Vec::new();
    for binary in files
        .iter()
        .filter(|path| **path == "tests/it.rs" || path.ends_with("/tests/it.rs"))
    {
        binaries += 1;
        let dir = &binary[..binary.len() - "it.rs".len()];
        let source = std::fs::read_to_string(root.join(binary))
            .unwrap_or_else(|error| panic!("{binary}: {error}"));
        let declared: BTreeSet<&str> = source
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                line.strip_prefix("pub mod ")
                    .or_else(|| line.strip_prefix("mod "))?
                    .strip_suffix(';')
            })
            .collect();
        for file in &files {
            let Some(name) = file
                .strip_prefix(dir)
                .and_then(|rest| rest.strip_suffix(".rs"))
            else {
                continue;
            };
            if name != "it" && !name.contains('/') && !declared.contains(name) {
                undeclared.push(*file);
            }
        }
    }
    assert!(binaries > 1, "found {binaries} tests/it.rs in the checkout");
    assert!(
        undeclared.is_empty(),
        "suite files no tests/it.rs declares, so cargo never compiles them: {undeclared:?}"
    );
}
