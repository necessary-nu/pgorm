//! The user guide's shape: its chapter list, the modules that include the
//! chapters, and the examples in them.
//!
//! The examples themselves run as pgorm's doctests (`cargo test --doc -p
//! pgorm guide`); what is checked here is what a doctest run cannot see: that
//! no chapter is missing or out of order, that no example is skipped, and
//! that each chapter names the features it is there to teach.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The examples a doctest compiles but cannot run, per chapter, each with
/// the reason it cannot.
const NO_RUN: &[(&str, usize, &str)] = &[];

struct Chapter {
    id: String,
    title: String,
    file: String,
}

fn guide_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guide")
}

fn chapters() -> Vec<Chapter> {
    let path = guide_dir().join("chapters.json");
    let text = std::fs::read_to_string(&path).expect("docs/guide/chapters.json is readable");
    let list: Value = serde_json::from_str(&text).expect("docs/guide/chapters.json is JSON");
    list["chapters"]
        .as_array()
        .expect("chapters.json holds a `chapters` array")
        .iter()
        .map(|chapter| {
            let field = |name: &str| {
                chapter[name]
                    .as_str()
                    .unwrap_or_else(|| panic!("a chapter has no string `{name}`: {chapter}"))
                    .to_owned()
            };
            Chapter {
                id: field("id"),
                title: field("title"),
                file: field("file"),
            }
        })
        .collect()
}

fn read_chapter(chapter: &Chapter) -> String {
    std::fs::read_to_string(guide_dir().join(&chapter.file)).unwrap_or_else(|err| {
        panic!(
            "chapter {} ({}) is unreadable: {err}",
            chapter.id, chapter.file
        )
    })
}

/// Each fenced block of a chapter: its info string and its lines.
fn fences(text: &str) -> Vec<(String, Vec<String>)> {
    let mut blocks = Vec::new();
    let mut open: Option<(String, Vec<String>)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        match open.take() {
            None => {
                if let Some(info) = trimmed.strip_prefix("```") {
                    open = Some((info.trim().to_owned(), Vec::new()));
                }
            }
            Some((info, mut lines)) => {
                if trimmed.starts_with("```") {
                    blocks.push((info, lines));
                } else {
                    lines.push(line.to_owned());
                    open = Some((info, lines));
                }
            }
        }
    }
    assert!(open.is_none(), "a fenced block is never closed");
    blocks
}

/// Whether rustdoc reads a block with this info string as Rust.
fn is_rust(info: &str) -> bool {
    const OTHER: &[&str] = &[
        "text",
        "sql",
        "toml",
        "sh",
        "console",
        "python",
        "javascript",
        "typescript",
        "json",
    ];
    info.split(',')
        .map(str::trim)
        .all(|word| !OTHER.contains(&word))
}

// [spec:pgorm:req:guide.chapters/test]    the list, the files, their titles and the modules agree
#[test]
fn chapter_list_matches_files_titles_and_modules() {
    let chapters = chapters();
    assert!(!chapters.is_empty(), "the guide lists no chapter");

    for chapter in &chapters {
        let text = read_chapter(chapter);
        let first = text.lines().next().unwrap_or_default();
        assert_eq!(
            first,
            format!("# {}", chapter.title),
            "chapter {} must open with its title",
            chapter.id
        );
        assert!(
            chapter
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "chapter id {} is not kebab-case",
            chapter.id
        );
    }

    let guide_rs =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/guide.rs"))
            .expect("src/guide.rs is readable");
    let mut included = Vec::new();
    let mut pending: Option<String> = None;
    for line in guide_rs.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#[doc = include_str!(\"../docs/guide/") {
            let file = rest
                .strip_suffix("\")]")
                .expect("an include names its file and closes");
            pending = Some(file.to_owned());
        } else if let Some(rest) = line.strip_prefix("pub mod ") {
            let module = rest.strip_suffix(" {}").expect("a chapter module is empty");
            let file = pending
                .take()
                .expect("every chapter module includes its chapter");
            included.push((module.to_owned(), file));
        }
    }
    let expected: Vec<(String, String)> = chapters
        .iter()
        .map(|chapter| (chapter.id.replace('-', "_"), chapter.file.clone()))
        .collect();
    assert_eq!(
        included, expected,
        "src/guide.rs includes the listed chapters in order"
    );

    let mut on_disk: Vec<String> = std::fs::read_dir(guide_dir())
        .expect("docs/guide is a directory")
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".md") && name != "README.md")
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = chapters
        .iter()
        .map(|chapter| chapter.file.clone())
        .collect();
    listed.sort();
    assert_eq!(
        on_disk, listed,
        "every chapter file under docs/guide is listed, and no other"
    );

    let index = std::fs::read_to_string(guide_dir().join("README.md"))
        .expect("docs/guide/README.md is readable");
    let linked: Vec<String> = index
        .lines()
        .filter(|line| line.starts_with("| "))
        .filter_map(|line| {
            let target = line.split("](self::").nth(1)?;
            Some(target.split(')').next()?.to_owned())
        })
        .collect();
    assert_eq!(
        linked,
        expected
            .iter()
            .map(|(module, _)| module.clone())
            .collect::<Vec<_>>(),
        "the guide's index links every chapter, in order"
    );
}

// [spec:pgorm:req:guide.examples/test]    no example is skipped, each asserts, and each scratch database is dropped
#[test]
fn every_example_runs_and_asserts() {
    for chapter in chapters() {
        let text = read_chapter(&chapter);
        let mut no_run = 0;
        for (info, lines) in fences(&text) {
            if !is_rust(&info) {
                continue;
            }
            let words: Vec<&str> = info.split(',').map(str::trim).collect();
            assert!(
                !words.contains(&"ignore"),
                "chapter {} has an `ignore` example",
                chapter.id
            );
            assert!(
                !words.contains(&"should_panic"),
                "chapter {} has a `should_panic` example",
                chapter.id
            );
            if words.contains(&"no_run") {
                no_run += 1;
            }
            if words.contains(&"compile_fail") {
                continue;
            }
            let body = lines.join("\n");
            assert!(
                body.contains("assert"),
                "an example in chapter {} asserts nothing:\n{body}",
                chapter.id
            );
            if body.contains("Scratch::") {
                assert!(
                    body.contains(".finish().await"),
                    "an example in chapter {} never drops its scratch database:\n{body}",
                    chapter.id
                );
            }
        }
        let allowed = NO_RUN
            .iter()
            .find(|(id, _, _)| *id == chapter.id)
            .map_or(0, |(_, count, _)| *count);
        assert_eq!(
            no_run, allowed,
            "chapter {} has {no_run} `no_run` examples and NO_RUN allows {allowed}",
            chapter.id
        );
    }
}

/// What each chapter has to name, beyond the derives and the connection
/// methods the guide as a whole names: the features `guide.rust` gives that
/// chapter, each by the name a reader would look it up by.
const CHAPTER_NAMES: &[(&str, &[&str])] = &[
    (
        "getting-started",
        &["connect", "DatabasePool", "DatabaseConnection", "pg-19"],
    ),
    (
        "entities",
        &[
            "primary_key",
            "belongs_to",
            "has_many",
            "ActiveModelBehavior",
            "hooks",
        ],
    ),
    (
        "writing",
        &[
            "set",
            "Insert::one",
            "Insert::many",
            "exec_returning_pk",
            "exec_returning_pks",
            "exec_returning_model",
            "exec_returning_models",
            "Update::one",
            "Update::many",
            "Delete::one",
            "Delete::many",
            "OnConflict",
            "MERGE",
        ],
    ),
    (
        "reading",
        &[
            "find",
            "filter",
            "into_tuple",
            "order_by_asc",
            "limit",
            "offset",
        ],
    ),
    ("pagination", &["paginate", "cursor_by", "stream"]),
    ("relations", &["Related", "load_one", "load_many", "graph"]),
    (
        "transactions",
        &[
            "transaction",
            "transaction_with",
            "transaction_with_retry",
            "RollbackFailed",
        ],
    ),
    ("errors", &["SqlError", "sql_error"]),
    ("raw-sql", &["sql!", "prql!"]),
    (
        "query-builder",
        &[
            "Query::select",
            "Query::insert",
            "Query::update",
            "Query::delete",
        ],
    ),
    ("pipeline", &["pipeline"]),
    (
        "values",
        &[
            "Decimal",
            "Json",
            "Range",
            "Multirange",
            "CreatedRange",
            "Interval",
            "money",
        ],
    ),
    (
        "schema",
        &[
            "create_table_from_entity",
            "create_enum_from_entity",
            "create_index_from_entity",
            "create_comments_from_entity",
        ],
    ),
    ("disjoint", &["Disjoint", "create_disjoint"]),
    ("one-of", &["OneOf", "create_one_of"]),
    (
        "copy",
        &["copy_in", "copy_out", "copy_in_rows", "copy_out_rows"],
    ),
    ("listen-notify", &["listener", "notify"]),
    (
        "statement-caches",
        &["clear_statement_caches", "statement_cache_channel"],
    ),
    ("configuration", &["connect_with", "ManagerConfig"]),
    ("metrics", &["InstrumentedPool", "MetricsCollector"]),
    ("migrations", &["MigratorTrait", "MigrationTrait"]),
    ("codegen", &["entities_from_sql"]),
    ("postgres-releases", &["pg-19", "POSTGRES_TARGET"]),
    ("bindings", &["Python", "JavaScript"]),
];

/// The names a comma-separated `{ .. }` list holds, from the line in `source`
/// that opens with `opening`.
fn braced_names(source: &str, opening: &str) -> Vec<String> {
    let start = source
        .find(opening)
        .unwrap_or_else(|| panic!("`{opening}` is not in the source"));
    let rest = &source[start + opening.len()..];
    let end = rest.find('}').expect("the list closes");
    rest[..end]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

// [spec:pgorm:req:guide.rust/test]    every derive, every connection method and each chapter's features are named
#[test]
fn guide_names_every_feature() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let chapters = chapters();
    let text: Vec<(String, String)> = chapters
        .iter()
        .map(|chapter| (chapter.id.clone(), read_chapter(chapter)))
        .collect();
    let whole: String = text.iter().map(|(_, body)| body.as_str()).collect();

    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("src/lib.rs is readable");
    let mut derives = braced_names(&lib, "pub use pgorm_macros::{");
    derives.extend([
        "SqlName".to_owned(),
        "StaticName".to_owned(),
        "MigrationName".to_owned(),
    ]);
    for derive in derives {
        assert!(
            whole.contains(&format!("`{derive}`")) || whole.contains(&format!("derive({derive})")),
            "the guide never names the `{derive}` derive"
        );
    }

    let connection = std::fs::read_to_string(root.join("src/database/connection.rs"))
        .expect("src/database/connection.rs is readable");
    for line in connection.lines() {
        let Some(rest) = line.trim().strip_prefix("async fn ") else {
            continue;
        };
        let method: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        assert!(
            whole.contains(&method),
            "the guide never names ConnectionTrait::{method}"
        );
    }

    for (id, names) in CHAPTER_NAMES {
        let (_, body) = text
            .iter()
            .find(|(chapter, _)| chapter == id)
            .unwrap_or_else(|| panic!("CHAPTER_NAMES lists {id}, which chapters.json does not"));
        for name in *names {
            assert!(body.contains(name), "chapter {id} never names {name}");
        }
    }
    assert_eq!(
        CHAPTER_NAMES.len(),
        chapters.len(),
        "every chapter has its names listed"
    );
}
