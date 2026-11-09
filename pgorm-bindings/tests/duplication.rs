//! The two bindings' sources hold no large block alike: what both need is
//! written once in this crate, and each binding keeps only its language's
//! glue ([spec:pgorm:req:bindings.shared-runtime]).
//!
//! Each binding's sources are read as Rust tokens, their comments, their
//! whitespace and their test modules left out, and every run of identical
//! tokens one binding's file shares with the other's is measured. A run as
//! long as [`LIMIT`] fails: it is logic copied rather than called.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

/// The longest run of identical tokens the two bindings may share. Language
/// glue is shorter: the longest the bindings hold alike are a registry
/// method's generic bounds over pgorm-bindings' own and the list of DDL
/// statements a node can hold, both near 80; a function body written twice
/// is longer.
const LIMIT: usize = 100;

/// The shortest run the scan seeds a match with: shorter runs are counted
/// only as part of a longer one.
const SEED: usize = 24;

/// One token and the line it starts on. Two tokens are the same token when
/// their text is, wherever each stands.
#[derive(Debug, Clone)]
struct Token {
    text: String,
    line: usize,
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for Token {}

impl std::hash::Hash for Token {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

/// `source` as tokens: identifiers, literals and punctuation, with no
/// comment and no whitespace.
fn tokens(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let (mut at, mut line) = (0, 1);
    let mut push = |text: String, line: usize| out.push(Token { text, line });
    while at < chars.len() {
        let c = chars[at];
        let next = chars.get(at + 1).copied();
        if c == '\n' {
            line += 1;
            at += 1;
        } else if c.is_whitespace() {
            at += 1;
        } else if c == '/' && next == Some('/') {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while at < chars.len() {
                if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
                    depth += 1;
                    at += 2;
                } else if chars[at] == '*' && chars.get(at + 1) == Some(&'/') {
                    depth -= 1;
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[at] == '\n' {
                        line += 1;
                    }
                    at += 1;
                }
            }
        } else if c == 'r' && (next == Some('"') || next == Some('#')) && raw_string(&chars, at) {
            let start = (at, line);
            let mut hashes = 0;
            at += 1;
            while chars.get(at) == Some(&'#') {
                hashes += 1;
                at += 1;
            }
            at += 1;
            loop {
                match chars.get(at) {
                    None => break,
                    Some('"') if (1..=hashes).all(|k| chars.get(at + k) == Some(&'#')) => {
                        at += 1 + hashes;
                        break;
                    }
                    Some('\n') => {
                        line += 1;
                        at += 1;
                    }
                    Some(_) => at += 1,
                }
            }
            push(chars[start.0..at].iter().collect(), start.1);
        } else if c == '"' {
            let start = (at, line);
            at += 1;
            while at < chars.len() && chars[at] != '"' {
                if chars[at] == '\\' {
                    at += 1;
                }
                if chars.get(at) == Some(&'\n') {
                    line += 1;
                }
                at += 1;
            }
            at += 1;
            push(
                chars[start.0..at.min(chars.len())].iter().collect(),
                start.1,
            );
        } else if c == '\'' && char_literal(&chars, at) {
            let start = at;
            at += 1;
            if chars.get(at) == Some(&'\\') {
                at += 1;
            }
            at += 1;
            while at < chars.len() && chars[at] != '\'' {
                at += 1;
            }
            at += 1;
            push(chars[start..at.min(chars.len())].iter().collect(), line);
        } else if c.is_alphanumeric() || c == '_' || c == '\'' {
            let start = at;
            at += 1;
            while at < chars.len() && (chars[at].is_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            push(chars[start..at].iter().collect(), line);
        } else {
            push(c.to_string(), line);
            at += 1;
        }
    }
    out
}

/// Whether the `r` at `at` opens a raw string rather than ending a name.
fn raw_string(chars: &[char], at: usize) -> bool {
    if at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '_') {
        return false;
    }
    let mut k = at + 1;
    while chars.get(k) == Some(&'#') {
        k += 1;
    }
    chars.get(k) == Some(&'"')
}

/// Whether the quote at `at` opens a character literal rather than a
/// lifetime.
fn char_literal(chars: &[char], at: usize) -> bool {
    matches!(
        (chars.get(at + 1), chars.get(at + 2)),
        (Some('\\'), _) | (Some(_), Some('\''))
    )
}

/// A binding's production sources: every `.rs` file under `root` but its
/// test files, each cut at its test module.
fn sources(root: &Path) -> Vec<(PathBuf, Vec<Token>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let name = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned());
                if name.is_some_and(|name| name == "tests" || name.ends_with("_tests")) {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let production = match text.find("\n#[cfg(test)]\nmod ") {
                    Some(cut) => &text[..cut],
                    None => &text,
                };
                files.push((path, tokens(production)));
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// A run of identical tokens one file of each binding shares.
#[derive(Debug)]
struct Shared {
    length: usize,
    python: (PathBuf, usize, usize),
    napi: (PathBuf, usize, usize),
}

/// Every maximal run of at least [`SEED`] identical tokens `python` and
/// `napi` share, longest first.
fn shared(python: &[(PathBuf, Vec<Token>)], napi: &[(PathBuf, Vec<Token>)]) -> Vec<Shared> {
    let mut seeds: HashMap<&[Token], Vec<(usize, usize)>> = HashMap::new();
    for (file, (_, tokens)) in python.iter().enumerate() {
        for at in 0..tokens.len().saturating_sub(SEED - 1) {
            seeds
                .entry(&tokens[at..at + SEED])
                .or_default()
                .push((file, at));
        }
    }
    let mut found = Vec::new();
    for (path, tokens) in napi {
        let mut seen = HashSet::new();
        for at in 0..tokens.len().saturating_sub(SEED - 1) {
            let Some(starts) = seeds.get(&tokens[at..at + SEED]) else {
                continue;
            };
            for &(file, start) in starts {
                let theirs = &python[file].1;
                if start > 0 && at > 0 && theirs[start - 1].text == tokens[at - 1].text {
                    continue;
                }
                if !seen.insert((file, start)) {
                    continue;
                }
                let mut length = 0;
                while at + length < tokens.len()
                    && start + length < theirs.len()
                    && theirs[start + length].text == tokens[at + length].text
                {
                    length += 1;
                }
                found.push(Shared {
                    length,
                    python: (
                        python[file].0.clone(),
                        theirs[start].line,
                        theirs[start + length - 1].line,
                    ),
                    napi: (path.clone(), tokens[at].line, tokens[at + length - 1].line),
                });
            }
        }
    }
    found.sort_by_key(|run| std::cmp::Reverse(run.length));
    found
}

/// The repository root, which the scan's report names files from.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn binding(name: &str) -> PathBuf {
    root().join(name).join("src")
}

fn shown(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .display()
        .to_string()
}

// [spec:pgorm:req:bindings.shared-runtime/test]    neither binding holds a block of the other's
// as long as LIMIT tokens: what both need is written once, in pgorm-bindings
#[test]
fn the_bindings_share_no_large_block() {
    let python = sources(&binding("pgorm-python"));
    let napi = sources(&binding("pgorm-napi"));
    assert!(
        !python.is_empty() && !napi.is_empty(),
        "both bindings' sources are read"
    );
    let over: Vec<String> = shared(&python, &napi)
        .into_iter()
        .filter(|run| run.length >= LIMIT)
        .map(|run| {
            format!(
                "{} tokens: {}:{}-{} and {}:{}-{}",
                run.length,
                shown(&run.python.0),
                run.python.1,
                run.python.2,
                shown(&run.napi.0),
                run.napi.1,
                run.napi.2
            )
        })
        .collect();
    assert!(
        over.is_empty(),
        "the bindings hold the same logic twice; move it into pgorm-bindings:\n{}",
        over.join("\n")
    );
}

// [spec:pgorm:req:bindings.shared-runtime/test]    the scan reads code, not comments, and finds
// a block two files share however each is laid out
#[test]
fn a_shared_block_is_found_however_laid_out() {
    let body = (0..40)
        .map(|n| format!("let value_{n} = compute({n}, \"text {n}\");"))
        .collect::<Vec<_>>();
    let python = vec![(
        PathBuf::from("python.rs"),
        tokens(&format!("// glue\nfn python() {{\n{}\n}}", body.join("\n"))),
    )];
    let napi = vec![(
        PathBuf::from("napi.rs"),
        tokens(&format!(
            "/* other glue */ fn napi() {{ {} }}",
            body.join(" ")
        )),
    )];
    let runs = shared(&python, &napi);
    assert!(
        runs.first().is_some_and(|run| run.length > 40 * 10),
        "{runs:?}"
    );
    assert!(tokens("// a comment\n/* and /* nested */ one */ x").len() == 1);
    assert_eq!(
        tokens(r##"r#"a "quoted" string"# 'c' 'a"##)
            .into_iter()
            .map(|token| token.text)
            .collect::<Vec<_>>(),
        [r##"r#"a "quoted" string"#"##, "'c'", "'a"]
    );
}
