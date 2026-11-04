//! The post-compilation parameter census: what the emitted SQL still asks
//! for, and the values compacted to match.
//!
//! The binder mints a `$N` for every bound value, but prqlc's optimizer may
//! prune an expression nothing reads — a derived column a later `select`
//! drops — and the placeholder vanishes with it while the value stays. The
//! server refuses that mismatch at bind time, so [`prune`] re-counts the
//! placeholders the emitted SQL actually carries, discards the orphaned
//! values and renumbers the survivors contiguously, rewriting the SQL and
//! compacting the values in one pass.
//!
//! The count is a lexer census, not a text scan: `pg_query::scan` — the
//! PostgreSQL lexer pgorm already links — hands back `PARAM` tokens with
//! byte extents, so a `$1` sitting inside a string literal
//! (`filter(note.eq("costs $1"))` renders as `'costs $1'`) is never
//! miscounted, and the extents are exactly what the rewrite needs. Reading
//! the finished SQL is complete here where it has to be defended in `prql!`
//! ([spec:pgorm:sem:macros.prql.census]): the pipeline offers no raw-SQL
//! hole ([spec:pgorm:req:pipeline.surface+4] cuts s-strings), and the one
//! text it hands the compiler to pass through untouched is a string literal
//! it rendered itself ([spec:pgorm:req:pipeline.params+4]), which the lexer
//! reads as a single constant. Nor can a name lex as one: the identifier
//! screen refuses every name that begins with `$`
//! ([spec:pgorm:req:pipeline.errors+4]), which the compiler would otherwise
//! write bare. So every placeholder in the emitted text is a `Param` node
//! the binder minted, and the census can only ever find a subset of the
//! minted numbers — never a foreign `$N` with no value behind it.

use std::collections::BTreeSet;
use std::ops::Range;

use pgorm_query::{Value, Values};

/// Discard values whose placeholders the optimizer pruned and renumber the
/// surviving `$N` contiguously from `$1`, so position `N` in the SQL is
/// position `N` in the values again.
///
/// A repeated placeholder keeps its value once; every occurrence is
/// rewritten to the same new number. When every minted placeholder survived
/// — or when the census cannot be trusted (unscannable text, a number
/// outside what the binder minted; both unreachable from this module's own
/// output) — the statement passes through unchanged for the server to
/// judge, so this function never panics and never invents a binding.
// [spec:pgorm:req:pipeline.params+4]
pub(super) fn prune(sql: String, values: Vec<Value>) -> (String, Values) {
    if values.is_empty() {
        return (sql, Values(values));
    }
    match renumbered(&sql, values.len()) {
        Some((rewritten, kept)) => {
            let mut slots: Vec<Option<Value>> = values.into_iter().map(Some).collect();
            let survivors = kept
                .into_iter()
                .filter_map(|number| slots[number - 1].take())
                .collect();
            (rewritten, Values(survivors))
        }
        None => (sql, Values(values)),
    }
}

/// The rewritten SQL and the surviving placeholder numbers in ascending
/// order, or `None` when nothing needs to change.
fn renumbered(sql: &str, minted: usize) -> Option<(String, Vec<usize>)> {
    let occurrences = census(sql)?;
    let survivors: BTreeSet<usize> = occurrences.iter().map(|(_, number)| *number).collect();
    if survivors
        .iter()
        .any(|&number| number == 0 || number > minted)
    {
        return None;
    }
    if survivors.len() == minted {
        // Distinct, in range, and as many as were minted: exactly $1..$N.
        return None;
    }

    let mut rewritten = String::with_capacity(sql.len());
    let mut cursor = 0;
    for (extent, number) in occurrences {
        let rank = survivors.range(..=number).count();
        rewritten.push_str(sql.get(cursor..extent.start)?);
        rewritten.push('$');
        rewritten.push_str(&rank.to_string());
        cursor = extent.end;
    }
    rewritten.push_str(sql.get(cursor..)?);

    Some((rewritten, survivors.into_iter().collect()))
}

/// Every `PARAM` token in `sql`, in text order: its byte extent and number.
///
/// `None` means the text could not be lexed or a token was not the `$N`
/// shape the lexer promises — either way, no census to act on.
fn census(sql: &str) -> Option<Vec<(Range<usize>, usize)>> {
    let scanned = pg_query::scan(sql).ok()?;
    let mut occurrences = Vec::new();
    for token in &scanned.tokens {
        if token.token != pg_query::protobuf::Token::Param as i32 {
            continue;
        }
        let extent = usize::try_from(token.start).ok()?..usize::try_from(token.end).ok()?;
        let number = sql.get(extent.clone())?.strip_prefix('$')?.parse().ok()?;
        occurrences.push((extent, number));
    }
    Some(occurrences)
}
