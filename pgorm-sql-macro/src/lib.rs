//! Compile-time query validation for pgorm's raw-SQL escape hatches.
//!
//! Two function-like macros live here for callers: [`sql!`] holds a raw SQL
//! string literal to the PostgreSQL grammar, and [`prql!`] compiles a PRQL
//! string literal down to PostgreSQL SQL with its placeholder arity checked
//! against the arguments given. A third, hidden one is the compile-time half
//! of pgorm-migration's `sql_migration!`, which holds a migration's `.sql`
//! file to the same grammar. The crate is a sibling of `pgorm-macros` rather
//! than more macros inside it because the entity derives have no use for
//! either the libpg_query oracle or the PRQL compiler.
#![warn(missing_docs)]
#![deny(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::todo,
    clippy::unreachable,
    clippy::unwrap_used
)]

extern crate proc_macro;

mod migration;
mod prql;
mod sql;

use proc_macro::TokenStream;
use quote::quote;
use syn::{LitStr, parse_macro_input};

/// Hold a raw SQL string literal to the PostgreSQL grammar at compile time.
///
/// The literal is parsed by libpg_query — the PostgreSQL server's own parser —
/// while the crate that wrote it is compiled. A literal the grammar accepts
/// expands to itself, unchanged, as a `&'static str`; a literal the grammar
/// rejects is a compile error on the literal carrying the parser's own message.
///
/// ```
/// use pgorm_sql_macro::sql;
///
/// const RECENT: &str = sql!("SELECT id, name FROM cake WHERE id > $1 ORDER BY id");
/// assert_eq!(RECENT, "SELECT id, name FROM cake WHERE id > $1 ORDER BY id");
///
/// // A `;`-separated script is checked statement by statement.
/// const SETUP: &str = sql!("CREATE TABLE cake (id int); INSERT INTO cake VALUES (1);");
/// assert!(SETUP.contains("INSERT INTO cake"));
/// ```
///
/// `sql!("SELECT * FORM cake")` does not compile: the parser reports
/// `syntax error at or near "cake"`, and the macro reports it back on the
/// literal with a caret under the offending column.
///
/// The check is syntax only. libpg_query carries no catalog, so a well-formed
/// statement against tables and columns that do not exist passes happily — this
/// rules out typos in SQL, not mistakes about the schema.
// [spec:pgorm:def:macros.sql+6]
// [spec:pgorm:req:macros.sql.reject]    non-literal input is `syn`'s own refusal
#[proc_macro]
pub fn sql(input: TokenStream) -> TokenStream {
    let literal = parse_macro_input!(input as LitStr);

    match sql::validate(&literal) {
        Ok(()) => quote!(#literal).into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Compile a PRQL string literal to PostgreSQL SQL at build time, expanding
/// to `(&'static str, pgorm::Values)`.
///
/// The literal is compiled by prqlc while your crate compiles; the emitted
/// SQL is then held to libpg_query — the same oracle behind [`sql!`] — and
/// its `$N` placeholders are counted against the arguments that follow the
/// literal. What lands in your code is the finished SQL as a `&'static str`
/// and the arguments converted via `Into<pgorm::Value>`, in placeholder
/// order, ready for `SelectorRaw::from_statement` and the other raw-SQL
/// entry points.
///
/// ```
/// use pgorm_sql_macro::prql;
///
/// let min_total = 5_i64;
/// let (sql, values) = prql!("from invoice | filter total > $1 | take 5", min_total);
/// assert_eq!(sql, "SELECT * FROM invoice WHERE total > $1 LIMIT 5");
/// assert_eq!(values, pgorm::Values(vec![pgorm::Value::from(5_i64)]));
/// ```
///
/// A placeholder may be reused — `$1` twice in the query is still one
/// argument, bound once:
///
/// ```
/// use pgorm_sql_macro::prql;
///
/// let (sql, values) = prql!(
///     "from invoice | filter total > $1 | filter subtotal < $1",
///     100_i64,
/// );
/// assert_eq!(sql, "SELECT * FROM invoice WHERE total > $1 AND subtotal < $1");
/// assert_eq!(values.0.len(), 1);
/// ```
///
/// Five mistakes are compile errors, not runtime surprises: PRQL the
/// compiler rejects (including `take $1` — PRQL refuses a parameterized
/// limit), emitted SQL the PostgreSQL grammar rejects (the way a broken
/// s-string surfaces), an argument count that does not match the
/// placeholders, and a placeholder sequence with a gap (`$1, $3` and no
/// `$2`). S-strings pass through as written — they are the escape hatch —
/// but the final SQL they land in is still validated.
///
/// The check ends where the catalog begins: whether `total` is a column and
/// whether `$1` accepts an `i64` are the server's questions, answered at
/// prepare time and by `VerifyTrait::verify` at runtime.
// [spec:pgorm:def:macros.prql+1]
// [spec:pgorm:req:macros.prql.reject]    non-literal input is `syn`'s own refusal
// [spec:pgorm:req:macros.prql.ceiling]    the limits are stated on the macro
#[proc_macro]
pub fn prql(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as prql::PrqlInput);

    match prql::expand(&input) {
        Ok(expansion) => expansion.into(),
        Err(error) => {
            // A combined error renders as several `compile_error!` statements;
            // the block keeps them legal in the expression position the tuple
            // would have occupied, so every span is reported.
            let errors = error.to_compile_error();
            quote!({ #errors }).into()
        }
    }
}

/// The compile-time half of pgorm-migration's `sql_migration!`, which is the
/// macro to call: it passes its own crate path ahead of the caller's tokens,
/// as `sql_migration!($crate, "path")`.
///
/// Expands to `$crate::SqlMigration::new(name, text)`, the name being the
/// file stem and the text the file's with CRLF line endings read as LF, after
/// holding that text to the grammar [`sql!`] uses. The path is resolved
/// against `CARGO_MANIFEST_DIR`.
// [spec:pgorm:def:macros.sql+6]    the hidden third macro
// [spec:pgorm:def:migration.sql-file]
#[doc(hidden)]
#[proc_macro]
pub fn sql_migration(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as migration::MigrationInput);

    match migration::expand(&input) {
        Ok(expansion) => expansion.into(),
        Err(error) => error.to_compile_error().into(),
    }
}
