//! Placeholder templates.
//!
//! A `$N` template's placeholder census is knowable the moment the template
//! meets the values it substitutes, so this module resolves the two against
//! each other up front and hands back a flat sequence of [`Segment`]s that
//! carries no indices at all. Rendering a resolved sequence is a walk over
//! text and values; there is nothing left to look up, and so nothing left to
//! be out of range.

use crate::error::{Error, Result, TemplateError};
use crate::expr::SimpleExpr;
use crate::token::{Token, Tokenizer};
use crate::value::Value;

/// One piece of a scanned template: literal text, or a reference to the `N`th
/// substitution as the template wrote it (1-based, unvalidated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Chunk {
    Text(String),
    Placeholder(usize),
}

/// One piece of a resolved template. The placeholder indices are gone: each
/// reference has already been paired with the value it names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Segment<T> {
    Text(String),
    Value(T),
}

/// Which `$` convention a scan follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grammar {
    /// Authored SQL templates: `$$` escapes a literal `$`, and a `$` that
    /// is neither an escape nor a placeholder index is refused, because a
    /// template is authored for this machinery and a stray `$` in one is a
    /// mistake rather than data.
    Template,
    /// Real SQL being re-inlined: only `$N` is a placeholder and every other
    /// token — `$$` included, which opens a dollar-quoted body in PostgreSQL —
    /// is reproduced verbatim.
    Sql,
}

/// Split `input` into literal text and placeholder references.
// [spec:pgorm:req:sql.render.custom-expr+4] (the `Template` grammar: `$$` escape, `$N` index)
// [spec:pgorm:sem:sql.render.inject+3] (the `Sql` grammar: only `$N`, everything else verbatim)
pub(crate) fn scan(input: &str, grammar: Grammar) -> Result<Vec<Chunk>> {
    let mark = "$";
    let tokens: Vec<Token> = match grammar {
        Grammar::Sql => Tokenizer::new(input).iter().collect(),
        Grammar::Template => Tokenizer::new_without_dollar_quoting(input)
            .iter()
            .collect(),
    };

    let mut offsets = Vec::with_capacity(tokens.len());
    let mut offset = 0usize;
    for token in &tokens {
        offsets.push(offset);
        offset += token.as_str().chars().count();
    }

    let mut chunks = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < tokens.len() {
        let Token::Punctuation(punct) = &tokens[i] else {
            text.push_str(tokens[i].as_str());
            i += 1;
            continue;
        };
        if punct.as_str() != mark {
            text.push_str(punct);
            i += 1;
            continue;
        }

        let next = tokens.get(i + 1);
        let index = match next {
            Some(Token::Unquoted(word)) => word.parse::<usize>().ok(),
            _ => None,
        };
        if let Some(index) = index {
            if !text.is_empty() {
                chunks.push(Chunk::Text(std::mem::take(&mut text)));
            }
            chunks.push(Chunk::Placeholder(index));
            i += 2;
            continue;
        }

        let escaped = matches!(next, Some(Token::Punctuation(p)) if p.as_str() == mark);
        match grammar {
            Grammar::Template if escaped => {
                text.push_str(mark);
                i += 2;
            }
            Grammar::Template => {
                return Err(Error::Template {
                    template: input.to_owned(),
                    reason: TemplateError::MalformedPlaceholder {
                        position: offsets.get(i).copied().unwrap_or_default(),
                    },
                });
            }
            Grammar::Sql => {
                text.push_str(punct);
                i += 1;
            }
        }
    }
    if !text.is_empty() {
        chunks.push(Chunk::Text(text));
    }
    Ok(chunks)
}

/// Check the census of `chunks` against `supplied`: the distinct placeholder
/// indices referenced must be exactly `1..=supplied`.
// [spec:pgorm:req:sql.render.custom-expr+4]
// [spec:pgorm:sem:sql.render.inject+3]
fn census(input: &str, chunks: &[Chunk], supplied: usize) -> Result<()> {
    let mut referenced: Vec<usize> = chunks
        .iter()
        .filter_map(|chunk| match chunk {
            Chunk::Placeholder(index) => Some(*index),
            Chunk::Text(_) => None,
        })
        .collect();
    referenced.sort_unstable();
    referenced.dedup();

    let fail = |reason| Error::Template {
        template: input.to_owned(),
        reason,
    };

    for &index in &referenced {
        if index == 0 {
            return Err(fail(TemplateError::ZeroIndex));
        }
        if index > supplied {
            return Err(fail(TemplateError::IndexOutOfRange { index, supplied }));
        }
    }
    for index in 1..=supplied {
        if referenced.binary_search(&index).is_err() {
            return Err(fail(TemplateError::UnreferencedValue { index, supplied }));
        }
    }
    Ok(())
}

/// Scan `input` and pair every placeholder with the value it names, or refuse
/// the pairing. The result holds values, not indices.
// [spec:pgorm:req:sql.render.custom-expr+4]
// [spec:pgorm:sem:sql.render.inject+3]
pub(crate) fn resolve<T>(input: &str, grammar: Grammar, values: &[T]) -> Result<Vec<Segment<T>>>
where
    T: Clone,
{
    let chunks = scan(input, grammar)?;
    census(input, &chunks, values.len())?;

    chunks
        .into_iter()
        .map(|chunk| match chunk {
            Chunk::Text(text) => Ok(Segment::Text(text)),
            Chunk::Placeholder(index) => index
                .checked_sub(1)
                .and_then(|i| values.get(i))
                .map(|value| Segment::Value(value.clone()))
                .ok_or_else(|| Error::Template {
                    template: input.to_owned(),
                    reason: TemplateError::IndexOutOfRange {
                        index,
                        supplied: values.len(),
                    },
                }),
        })
        .collect()
}

// [spec:pgorm:req:sql.render.custom-expr+4/test]
// [spec:pgorm:sem:sql.render.inject+3/test]
#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn text(s: &str) -> Chunk {
        Chunk::Text(s.to_owned())
    }

    #[test]
    fn scans_placeholders_and_text() {
        assert_eq!(
            scan("6 = $1 * $2", Grammar::Template).unwrap(),
            vec![
                text("6 = "),
                Chunk::Placeholder(1),
                text(" * "),
                Chunk::Placeholder(2),
            ]
        );
    }

    #[test]
    fn escaped_dollar_is_literal_text() {
        assert_eq!(
            scan("$1 $$ $2", Grammar::Template).unwrap(),
            vec![Chunk::Placeholder(1), text(" $ "), Chunk::Placeholder(2),]
        );
    }

    #[test]
    fn quoted_placeholders_are_opaque() {
        assert_eq!(
            scan("'$1' = $1", Grammar::Template).unwrap(),
            vec![text("'$1' = "), Chunk::Placeholder(1)]
        );
    }

    #[test]
    fn template_refuses_a_bare_dollar() {
        assert_eq!(
            scan("a $ b", Grammar::Template).unwrap_err(),
            Error::Template {
                template: "a $ b".to_owned(),
                reason: TemplateError::MalformedPlaceholder { position: 2 },
            }
        );
    }

    #[test]
    fn template_refuses_a_trailing_dollar() {
        assert_eq!(
            scan("a $", Grammar::Template).unwrap_err(),
            Error::Template {
                template: "a $".to_owned(),
                reason: TemplateError::MalformedPlaceholder { position: 2 },
            }
        );
    }

    #[test]
    fn template_refuses_a_non_numeric_placeholder() {
        assert_eq!(
            scan("$abc", Grammar::Template).unwrap_err(),
            Error::Template {
                template: "$abc".to_owned(),
                reason: TemplateError::MalformedPlaceholder { position: 0 },
            }
        );
    }

    #[test]
    fn sql_grammar_passes_stray_dollars_through() {
        assert_eq!(
            scan("$$body$$ $1", Grammar::Sql).unwrap(),
            vec![text("$$body$$ "), Chunk::Placeholder(1)]
        );
    }

    #[test]
    fn resolve_pairs_repeated_references() {
        assert_eq!(
            resolve("$1 + $1", Grammar::Template, &["a"]).unwrap(),
            vec![
                Segment::Value("a"),
                Segment::Text(" + ".to_owned()),
                Segment::Value("a"),
            ]
        );
    }

    #[test]
    fn resolve_refuses_under_supply() {
        assert_eq!(
            resolve("$1 + $2", Grammar::Template, &["a"]).unwrap_err(),
            Error::Template {
                template: "$1 + $2".to_owned(),
                reason: TemplateError::IndexOutOfRange {
                    index: 2,
                    supplied: 1
                },
            }
        );
    }

    #[test]
    fn resolve_refuses_over_supply() {
        assert_eq!(
            resolve("$1", Grammar::Template, &["a", "b"]).unwrap_err(),
            Error::Template {
                template: "$1".to_owned(),
                reason: TemplateError::UnreferencedValue {
                    index: 2,
                    supplied: 2
                },
            }
        );
    }

    #[test]
    fn resolve_refuses_an_arity_hole() {
        assert_eq!(
            resolve("$1 + $3", Grammar::Template, &["a", "b", "c"]).unwrap_err(),
            Error::Template {
                template: "$1 + $3".to_owned(),
                reason: TemplateError::UnreferencedValue {
                    index: 2,
                    supplied: 3
                },
            }
        );
    }

    #[test]
    fn resolve_refuses_a_zero_index() {
        assert_eq!(
            resolve("$0", Grammar::Template, &["a"]).unwrap_err(),
            Error::Template {
                template: "$0".to_owned(),
                reason: TemplateError::ZeroIndex,
            }
        );
    }

    // [spec:pgorm:req:sql.render.template-pieces/test]
    #[test]
    fn pieces_take_their_values_between_them() {
        let template = SqlTemplate::from_pieces(
            &["coalesce(", ", ", ") > $$ $9 $$"],
            [SimpleExpr::Value(1.into()), SimpleExpr::Value(2.into())],
        )
        .unwrap();
        assert_eq!(
            template.segments,
            vec![
                Segment::Text("coalesce(".to_owned()),
                Segment::Value(SimpleExpr::Value(1.into())),
                Segment::Text(", ".to_owned()),
                Segment::Value(SimpleExpr::Value(2.into())),
                Segment::Text(") > $$ $9 $$".to_owned()),
            ]
        );
        assert_eq!(
            SqlTemplate::from_pieces(&["$2 + ", ""], [SimpleExpr::Value(1.into())]).unwrap_err(),
            Error::Template {
                template: "$2 + ".to_owned(),
                reason: TemplateError::MarkerInText { index: 2 },
            }
        );
        assert!(matches!(
            SqlTemplate::from_pieces(&["'", "'"], [SimpleExpr::Value(1.into())]),
            Err(Error::Template {
                reason: TemplateError::UnreferencedValue {
                    index: 1,
                    supplied: 1
                },
                ..
            })
        ));
    }

    #[test]
    fn resolve_accepts_a_template_with_no_placeholders() {
        assert_eq!(
            resolve::<&str>("now()", Grammar::Template, &[]).unwrap(),
            vec![Segment::Text("now()".to_owned())]
        );
    }
}

/// A validated SQL template and the expressions it substitutes, already
/// resolved against each other.
///
/// A template's placeholder census — which `$N` it names, and how many values
/// it therefore needs — is knowable the moment the template meets its values,
/// so [`SqlTemplate::new`] settles it there. What survives construction is a
/// flat sequence of literal text and resolved expressions carrying no indices
/// at all, which is why rendering a template cannot reach for a
/// substitution that was never supplied.
///
/// Two constructors read two different `$` grammars, because there are two
/// kinds of text a template is made of: [`SqlTemplate::new`] reads a template
/// authored for this machinery, where `$$` writes a literal `$`, and
/// [`SqlTemplate::from_sql`] reads real SQL, where `$$` opens a dollar-quoted
/// body. The grammar is a reading, not a property of the value: what either
/// constructor hands back is the same resolved sequence.
///
/// In expression position a template is reached through
/// [`Expr::template`](crate::Expr::template),
/// [`Expr::template_with_expr`](crate::Expr::template_with_expr) and
/// [`Expr::template_with_exprs`](crate::Expr::template_with_exprs); in relation
/// position it is the payload of
/// [`FromItem::Template`](crate::FromItem::Template).
// [spec:pgorm:req:sql.render.custom-expr+4]
#[derive(Debug, Clone, PartialEq)]
pub struct SqlTemplate {
    segments: Vec<Segment<SimpleExpr>>,
}

impl SqlTemplate {
    /// Pair `template` with `values`, or refuse the pair.
    ///
    /// The template is a `&'static str`: a template authored for this
    /// machinery is program text, as [`Expr::raw`](crate::Expr::raw)'s is, and
    /// text built at run time cannot become one. Real SQL that arrives at run
    /// time, a caller's own statement, is [`SqlTemplate::from_sql`]'s.
    ///
    /// The template is tokenized, so placeholder-like text inside quoted
    /// tokens is left alone. `$$` writes a literal `$`; `$N` names the `N`th
    /// value counting from 1, and may be written as many times as you like.
    /// Any other `$` — trailing, `$0`, `$abc`, or one standing before a space
    /// or punctuation — is a malformed placeholder and is refused.
    ///
    /// The distinct indices the template references MUST be exactly
    /// `1..=values.len()`: a reference past the end
    /// ([`TemplateError::IndexOutOfRange`](crate::error::TemplateError::IndexOutOfRange))
    /// and a value the template never names
    /// ([`TemplateError::UnreferencedValue`](crate::error::TemplateError::UnreferencedValue),
    /// which is also what an arity hole like `$1, $3` over three values
    /// reports) are both refused rather than silently rendered.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let paired = SqlTemplate::new("$1 * $2", [Expr::val(2).into(), Expr::val(3).into()]);
    /// assert!(paired.is_ok());
    ///
    /// let short = SqlTemplate::new("$1 * $2", [Expr::val(2).into()]);
    /// assert!(short.is_err());
    /// ```
    // [spec:pgorm:req:sql.render.custom-expr+4]
    pub fn new<I>(template: &'static str, values: I) -> Result<Self>
    where
        I: IntoIterator<Item = SimpleExpr>,
    {
        let values: Vec<SimpleExpr> = values.into_iter().collect();
        Ok(Self {
            segments: resolve(template, Grammar::Template, &values)?,
        })
    }

    /// Pair real SQL — text this crate did not author — with the values its
    /// `$N` markers number, or refuse the pair.
    ///
    /// This is [`SqlTemplate::new`]'s sibling for foreign text, and it differs
    /// in exactly one thing: the `$` grammar. Here only `$N` is a marker and
    /// every other `$` is reproduced verbatim, `$$` included, because in real
    /// SQL `$$` opens a dollar-quoted body rather than standing in for a
    /// literal `$` — the same reading
    /// [`inject_parameters`](crate::inject_parameters) does. Comments lex as
    /// space and quoted regions are opaque, so a `$99` written inside a
    /// comment, a string literal, an `E'…'` string or a dollar-quoted body is
    /// text and binds nothing.
    ///
    /// The census is the same and equally exact: the distinct indices the SQL
    /// references MUST be `1..=values.len()`, so a marker with no value behind
    /// it and a value the SQL never reads are both refused here rather than
    /// discovered by the server.
    ///
    /// Each marker is paired with its value up front, so a template placed in
    /// [relation position](crate::FromItem::Template) renumbers into the
    /// enclosing statement's parameter space when it renders, rather than
    /// requiring that the fragment's numbering happen to be free.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// // `$$ … $$` is a dollar-quoted body, and the `$1` inside it is text.
    /// let quoted = SqlTemplate::from_sql("SELECT $1, $$ $1 $$", [1i32.into()]);
    /// assert!(quoted.is_ok());
    ///
    /// // A marker with no value behind it is refused.
    /// let unbound = SqlTemplate::from_sql("SELECT $1, $2", [1i32.into()]);
    /// assert!(unbound.is_err());
    /// ```
    // [spec:pgorm:req:sql.render.custom-expr+4]
    pub fn from_sql<T, I>(sql: T, values: I) -> Result<Self>
    where
        T: Into<String>,
        I: IntoIterator<Item = Value>,
    {
        let sql = sql.into();
        let values: Vec<SimpleExpr> = values.into_iter().map(SimpleExpr::Value).collect();
        Ok(Self {
            segments: resolve(&sql, Grammar::Sql, &values)?,
        })
    }

    /// Pair the pieces of a template literal with the values written between
    /// them: `pieces[0]`, then `values[0]`, then `pieces[1]`, and so on, the
    /// shape of a Python t-string's or a JavaScript tagged template's parts.
    ///
    /// This is the door the language bindings' `template` uses, holding its
    /// text to their languages' template literals as far as the language
    /// allows, where Rust's own [`Expr::template`](crate::Expr::template)
    /// holds it to program text by its type. The pieces are SQL, read as
    /// [`from_sql`](SqlTemplate::from_sql) reads it, and the values go
    /// between them, so a piece cannot write a `$N` marker of its own: one
    /// that does is refused with
    /// [`TemplateError::MarkerInText`](crate::error::TemplateError::MarkerInText).
    /// A value that lands inside a quoted region or an identifier, where
    /// PostgreSQL would read no parameter, is refused by the census as a value
    /// nothing reads.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let short = SqlTemplate::from_pieces(&["char_length(", ") < 40"], [Expr::val("Fudge").into()]);
    /// assert!(short.is_ok());
    ///
    /// // `$1` in a piece is SQL text claiming a value the interpolations hold.
    /// let marker = SqlTemplate::from_pieces(&["$1 < ", ""], [Expr::val(1).into()]);
    /// assert!(marker.is_err());
    ///
    /// // A value inside a string literal is never read as a parameter.
    /// let quoted = SqlTemplate::from_pieces(&["'", "'"], [Expr::val(1).into()]);
    /// assert!(quoted.is_err());
    /// ```
    // [spec:pgorm:req:sql.render.template-pieces]
    pub fn from_pieces<S, I>(pieces: &[S], values: I) -> Result<Self>
    where
        S: AsRef<str>,
        I: IntoIterator<Item = SimpleExpr>,
    {
        let values: Vec<SimpleExpr> = values.into_iter().collect();
        let mut joined = String::new();
        for (index, piece) in pieces.iter().enumerate() {
            let piece = piece.as_ref();
            if let Some(Chunk::Placeholder(marker)) = scan(piece, Grammar::Sql)?
                .into_iter()
                .find(|chunk| matches!(chunk, Chunk::Placeholder(_)))
            {
                return Err(Error::Template {
                    template: piece.to_owned(),
                    reason: TemplateError::MarkerInText { index: marker },
                });
            }
            if index > 0 {
                joined.push_str(&format!("${index}"));
            }
            joined.push_str(piece);
        }
        Ok(Self {
            segments: resolve(&joined, Grammar::Sql, &values)?,
        })
    }

    pub(crate) fn segments(&self) -> &[Segment<SimpleExpr>] {
        &self.segments
    }
}
