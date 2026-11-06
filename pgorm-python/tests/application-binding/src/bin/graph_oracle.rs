//! Compare installed Python cursor rows against independent Rust graph cursors.

use _native::{account, graphs};
use pgorm::{ConnectionTrait, DatabaseConnection};
use serde_json::{Value, json};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn compare(db: &DatabaseConnection, report: &Value) -> Result {
    for sql in [
        "CREATE TYPE python_entities.\"Mood\" AS ENUM ('calm','busy')",
        "CREATE TABLE python_entities.accounts (id integer PRIMARY KEY, \"display name\" text NOT NULL, note text, version integer NOT NULL, mood python_entities.\"Mood\" NOT NULL)",
        "CREATE TABLE python_entities.notes (id integer PRIMARY KEY, account_id integer NOT NULL, body text NOT NULL)",
        "INSERT INTO python_entities.accounts VALUES (1,'one',NULL,1,'calm'),(2,'two',NULL,1,'calm'),(3,'three',NULL,1,'busy')",
        "INSERT INTO python_entities.notes VALUES (11,1,'first'),(12,1,'second'),(21,2,'third')",
    ] {
        db.execute(sql, &[]).await?;
    }
    let graph = graphs::optional(&["n".to_owned()]);
    let mut cases = Vec::new();
    macro_rules! case {
        ($name:literal, $column:expr, $($method:ident($($argument:expr),*)),+) => {{
            let mut cursor = graph.clone().cursor_by($column);
            $(cursor.$method($($argument),*);)+
            cases.push(($name, cursor));
        }};
    }
    case!("first", account::Column::Id, first(2));
    case!(
        "full_after",
        account::Column::Id,
        after_with((1, 11)),
        first(2)
    );
    case!("primary_after", account::Column::Id, after(1));
    case!(
        "before_last",
        account::Column::Id,
        before_with((2, 21)),
        last(1)
    );
    case!("descending", account::Column::Id, desc(), first(2));
    case!("replace_window", account::Column::Id, first(1), last(2));
    case!("empty", account::Column::Id, first(0));
    case!(
        "enum",
        account::Column::Mood,
        after_with(("calm", 1, 11)),
        first(2)
    );
    if report.as_object().map(|object| object.len()) != Some(cases.len()) {
        return Err("cursor report has missing or additional cases".into());
    }
    for (name, mut cursor) in cases {
        let rows = cursor.all(db).await?;
        let actual = json!(
            rows.into_iter()
                .map(|(root, joined)| (root.id, joined.map(|row| row.id)))
                .collect::<Vec<_>>()
        );
        if report[name] != actual {
            return Err(format!(
                "cursor {name}: Python {} differs from Rust {actual}",
                report[name]
            )
            .into());
        }
    }
    Ok(())
}

// [spec:pgorm:req:bindings.entity-graphs+4/test]
async fn run() -> Result {
    let report: Value =
        serde_json::from_slice(&std::fs::read(std::env::var("PGORM_GRAPH_REPORT")?)?)?;
    let config = std::env::var("PGORM_TEST_DSN")?.parse::<pgorm::Config>()?;
    let pool = pgorm::connect(config);
    let db = pool.get().await?;
    db.execute("CREATE SCHEMA python_entities", &[]).await?;
    let result = compare(&db, &report).await;
    let cleanup = db.execute("DROP SCHEMA python_entities CASCADE", &[]).await;
    result?;
    cleanup?;
    println!("Eight installed Python cursor cases match independent Rust SelectGraph cursors");
    Ok(())
}

fn main() -> Result {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}
