//! COPY's two statements, `COPY … FROM STDIN` and `COPY … TO STDOUT`, over
//! pgorm-query's `CopyFromStatement` and `CopyToStatement`.
//!
//! pgorm-query types a COPY by its format, so an option the format or the
//! direction does not take is a method its statement lacks. JavaScript names
//! the format with a string, so here each direction and format is a variant,
//! and a method the variant's statement lacks is refused with a
//! `ConstructionError` as it is called, never written for the server to
//! refuse. COPY takes no bind parameters: a statement renders through its
//! `Display`, a value in its `WHERE` or its query written as an escaped
//! literal, and it runs only through `copyIn` and `copyOut`, which carry its
//! data, never through a statement terminal.

use std::num::NonZeroU32;

use neon::prelude::*;
use pgorm::pgorm_query::{CopyIgnore, CopyLogVerbosity, CopySource, IntoCopySource};
use pgorm_bindings::DataFormat;

use super::{
    Node,
    args::{arg, choice, embedded, node, predicate_at, refuse, this},
    schema::{get, integer, names, object, pick, relation, text},
    write,
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("copyFromNew", copy_from_new),
    ("copyToNew", copy_to_new),
    ("copyColumns", copy_columns),
    ("copyFreeze", copy_freeze),
    ("copyWhere", copy_where),
    ("copyDelimiter", copy_delimiter),
    ("copyNullString", copy_null_string),
    ("copyDefaultString", copy_default_string),
    ("copyHeader", copy_header),
    ("copyHeaderMatch", copy_header_match),
    ("copyEncoding", copy_encoding),
    ("copyOnErrorIgnore", copy_on_error_ignore),
    ("copyQuote", copy_quote),
    ("copyEscape", copy_escape),
    ("copyForceNotNull", copy_force_not_null),
    ("copyForceNotNullAll", copy_force_not_null_all),
    ("copyForceNull", copy_force_null),
    ("copyForceNullAll", copy_force_null_all),
    ("copyForceQuote", copy_force_quote),
    ("copyForceQuoteAll", copy_force_quote_all),
];

/// The formats a COPY is made in, as JavaScript names them.
const FORMATS: &[&str] = &["text", "csv", "binary"];

pub(crate) use pgorm_bindings::{CopyFrom, CopyTo};

/// The COPY one JavaScript builder owns.
// [spec:pgorm:req:bindings.copy]
#[derive(Debug, Clone)]
pub(crate) enum CopyStatement {
    CopyFrom(CopyFrom),
    CopyTo(CopyTo),
}

impl CopyStatement {
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::CopyFrom(_) => "a COPY FROM STDIN",
            Self::CopyTo(_) => "a COPY TO STDOUT",
        }
    }

    fn format(&self) -> &'static str {
        match self {
            Self::CopyFrom(copy) => copy.format(),
            Self::CopyTo(copy) => copy.format(),
        }
    }

    fn rendered(&self) -> String {
        match self {
            Self::CopyFrom(copy) => copy.rendered(),
            Self::CopyTo(copy) => copy.rendered(),
        }
    }

    /// The SQL the statement runs as, which `inspect()` gives with no
    /// values. A value written into it as a literal cannot carry NUL, which
    /// the protocol's NUL-terminated statement text cannot hold.
    // [spec:pgorm:req:bindings.copy]
    pub(crate) fn sql<'cx>(&self, cx: &mut Cx<'cx>) -> NeonResult<String> {
        let sql = self.rendered();
        if sql.contains('\0') {
            return refuse(cx, "a COPY's statement text cannot contain NUL");
        }
        Ok(sql)
    }
}

impl From<CopyStatement> for Node {
    fn from(copy: CopyStatement) -> Self {
        Node::Copy(Box::new(copy))
    }
}

/// The COPY FROM at `index`, its statement text checked as `inspect()`
/// checks it, for `copyIn` to run.
pub(crate) fn copy_from_at(cx: &mut FunctionContext, index: usize) -> NeonResult<CopyFrom> {
    match copy_at(cx, index)? {
        CopyStatement::CopyFrom(copy) => Ok(copy),
        CopyStatement::CopyTo(_) => cx.throw_type_error(
            "copyIn runs a COPY FROM STDIN, from copyFrom(table, format); a COPY TO runs through copyOut",
        ),
    }
}

pub(crate) fn copy_to_at(cx: &mut FunctionContext, index: usize) -> NeonResult<CopyTo> {
    match copy_at(cx, index)? {
        CopyStatement::CopyTo(copy) => Ok(copy),
        CopyStatement::CopyFrom(_) => cx.throw_type_error(
            "copyOut runs a COPY TO STDOUT, from copyTo(source, format); a COPY FROM runs through copyIn",
        ),
    }
}

/// The COPY at `index`, in either direction, its statement text checked as
/// `inspect()` checks it.
pub(crate) fn copy_at(cx: &mut FunctionContext, index: usize) -> NeonResult<CopyStatement> {
    let value = arg(cx, index);
    let Some(Node::Copy(copy)) = node(cx, value) else {
        return cx.throw_type_error("expected a COPY this module built");
    };
    copy.sql(cx)?;
    Ok(*copy)
}

fn receiver(cx: &mut FunctionContext) -> NeonResult<CopyStatement> {
    match this(cx, 0)? {
        Node::Copy(copy) => Ok(*copy),
        other => cx.throw_type_error(format!("expected a COPY, got {}", other.describe())),
    }
}

/// The statements an option belongs to: its directions and its formats.
struct Takes {
    from: bool,
    to: bool,
    formats: &'static [&'static str],
}

const DELIMITED: Takes = Takes {
    from: true,
    to: true,
    formats: &["text", "csv"],
};

const DELIMITED_FROM: Takes = Takes {
    from: true,
    to: false,
    formats: &["text", "csv"],
};

const CSV: Takes = Takes {
    from: true,
    to: true,
    formats: &["csv"],
};

const CSV_FROM: Takes = Takes {
    from: true,
    to: false,
    formats: &["csv"],
};

const CSV_TO: Takes = Takes {
    from: false,
    to: true,
    formats: &["csv"],
};

const FROM: Takes = Takes {
    from: true,
    to: false,
    formats: FORMATS,
};

/// Refuse `option` on `copy`, which its direction or its format does not
/// take, saying which.
fn refused<T>(
    cx: &mut FunctionContext,
    option: &str,
    copy: &CopyStatement,
    takes: &Takes,
) -> NeonResult<T> {
    let message = match copy {
        CopyStatement::CopyFrom(_) if !takes.from => {
            format!("{option} belongs to COPY TO: a COPY FROM does not take it")
        }
        CopyStatement::CopyTo(_) if !takes.to => {
            format!("{option} belongs to COPY FROM: a COPY TO does not take it")
        }
        _ => format!(
            "{option} belongs to the {} format{}: this COPY is {}",
            takes.formats.join(" and "),
            if takes.formats.len() > 1 { "s" } else { "" },
            copy.format(),
        ),
    };
    refuse(cx, message)
}

/// Apply `$apply` to the statement `$s` of the variants listed, each of
/// which has the method it calls, and refuse every other variant as `$takes`
/// describes.
macro_rules! apply {
    ($cx:expr, $copy:expr, $option:expr, $takes:expr, [$($direction:ident :: $format:ident),+], |$s:ident| $apply:expr) => {
        match $copy {
            $(
                CopyStatement::$direction($direction::$format(mut $s)) => {
                    $apply;
                    Ok(CopyStatement::$direction($direction::$format($s)).into())
                }
            )+
            #[allow(unreachable_patterns)]
            other => refused($cx, $option, &other, &$takes),
        }
    };
}

/// The format the second argument names, one of [`FORMATS`].
fn data_format(cx: &mut FunctionContext) -> NeonResult<DataFormat> {
    Ok(match choice(cx, 1, "a COPY's format", FORMATS)? {
        "text" => DataFormat::Text,
        "csv" => DataFormat::Csv,
        _ => DataFormat::Binary,
    })
}

/// `copyFromNew(table, format)`: COPY into `table` from the client.
// [spec:pgorm:req:bindings.copy]
fn copy_from_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let table = arg(cx, 0);
    let table = relation(cx, table)?;
    let format = data_format(cx)?;
    Ok(CopyStatement::CopyFrom(CopyFrom::new(table, format)).into())
}

/// What a COPY TO reads: a table, named as a DDL statement names one, or
/// the rows of a `Select`, or of an INSERT, UPDATE, DELETE or MERGE through
/// its `RETURNING` list. A `Select` is final once read here, so its tie limit
/// and WINDOW clause are checked as any statement reading one checks them. A
/// write the binding would refuse to run is refused here as well, and so is
/// one with no `RETURNING` list, as pgorm-query's conversion into a
/// `CopySource` refuses it.
// [spec:pgorm:req:bindings.copy]
fn source<'cx>(
    cx: &mut FunctionContext<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<CopySource> {
    let statement = match node(cx, value) {
        None | Some(Node::Table(_)) => {
            return relation(cx, value).map(IntoCopySource::into_copy_source);
        }
        Some(statement) => statement,
    };
    if let Some(Err(reason)) = write::built(&statement) {
        return refuse(cx, reason);
    }
    let write = match statement {
        Node::Select(select) => return Ok(embedded(cx, select)?.into_copy_source()),
        Node::Insert(insert) => CopySource::try_from(insert.inner),
        Node::Update(update) => CopySource::try_from(update.inner),
        Node::Delete(delete) => CopySource::try_from(delete.inner),
        Node::Merge(merge) => CopySource::try_from(*merge),
        Node::PendingMerge(_) => {
            return refuse(cx, "a MERGE needs a WHEN arm before a COPY can read it");
        }
        other => {
            let what = other.describe();
            return cx.throw_type_error(format!(
                "a COPY TO reads a table, or a Select, Insert, Update, Delete or Merge, not {what}"
            ));
        }
    };
    match write {
        Ok(source) => Ok(source),
        Err(error) => refuse(cx, error.to_string()),
    }
}

/// `copyToNew(source, format)`: COPY a table's rows, or a statement's, to
/// the client.
// [spec:pgorm:req:bindings.copy]
fn copy_to_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let value = arg(cx, 0);
    let source = source(cx, value)?;
    let format = data_format(cx)?;
    Ok(CopyStatement::CopyTo(CopyTo::new(source, format)).into())
}

/// `copyColumns(copy, columns)`: the columns each row supplies, added to
/// those already named.
fn copy_columns(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let value = arg(cx, 1);
    let columns = names(cx, value, "a COPY's columns")?;
    apply!(
        cx,
        copy,
        "a column list",
        FROM,
        [CopyFrom::Text, CopyFrom::Csv, CopyFrom::Binary],
        |s| { s.columns(columns) }
    )
}

fn copy_freeze(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    apply!(
        cx,
        copy,
        "FREEZE",
        FROM,
        [CopyFrom::Text, CopyFrom::Csv, CopyFrom::Binary],
        |s| { s.freeze() }
    )
}

/// `copyWhere(copy, predicate)`: ANDed to the condition already there.
fn copy_where(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let predicate = predicate_at(cx, 1)?;
    apply!(
        cx,
        copy,
        "WHERE",
        FROM,
        [CopyFrom::Text, CopyFrom::Csv, CopyFrom::Binary],
        |s| { s.cond_where(predicate) }
    )
}

/// The one-character string at `index`, which pgorm-query takes as a `char`:
/// one code point, not NUL. Which characters the server accepts for an
/// option — one byte, no newline — stays the server's to decide.
fn character(cx: &mut FunctionContext, index: usize, what: &str) -> NeonResult<char> {
    let value = arg(cx, index);
    let text = text(cx, value, what)?;
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) => Ok(character),
        _ => refuse(cx, format!("{what} is a one-character string")),
    }
}

/// The string at `index`, written as a literal, which cannot carry NUL.
fn literal(cx: &mut FunctionContext, index: usize, what: &str) -> NeonResult<String> {
    let value = arg(cx, index);
    text(cx, value, what)
}

fn copy_delimiter(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let delimiter = character(cx, 1, "a delimiter")?;
    apply!(
        cx,
        copy,
        "DELIMITER",
        DELIMITED,
        [CopyFrom::Text, CopyFrom::Csv, CopyTo::Text, CopyTo::Csv],
        |s| s.delimiter(delimiter)
    )
}

fn copy_null_string(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let null = literal(cx, 1, "a NULL string")?;
    apply!(
        cx,
        copy,
        "NULL",
        DELIMITED,
        [CopyFrom::Text, CopyFrom::Csv, CopyTo::Text, CopyTo::Csv],
        |s| s.null_string(null.clone())
    )
}

fn copy_default_string(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let default = literal(cx, 1, "a DEFAULT string")?;
    apply!(
        cx,
        copy,
        "DEFAULT",
        DELIMITED_FROM,
        [CopyFrom::Text, CopyFrom::Csv],
        |s| { s.default_string(default.clone()) }
    )
}

fn copy_header(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    apply!(
        cx,
        copy,
        "HEADER",
        DELIMITED,
        [CopyFrom::Text, CopyFrom::Csv, CopyTo::Text, CopyTo::Csv],
        |s| s.header()
    )
}

fn copy_header_match(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    apply!(
        cx,
        copy,
        "HEADER match",
        DELIMITED_FROM,
        [CopyFrom::Text, CopyFrom::Csv],
        |s| { s.header_match() }
    )
}

fn copy_encoding(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let encoding = literal(cx, 1, "an encoding")?;
    apply!(
        cx,
        copy,
        "ENCODING",
        DELIMITED,
        [CopyFrom::Text, CopyFrom::Csv, CopyTo::Text, CopyTo::Csv],
        |s| s.encoding(encoding.clone())
    )
}

/// `copyOnErrorIgnore(copy, { rejectLimit, logVerbosity })`: `ON_ERROR
/// ignore`, its limit a whole number from 1 to 4,294,967,295 and its
/// reporting `"verbose"` or `"silent"`.
fn copy_on_error_ignore(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let options = object(cx, 1, &["rejectLimit", "logVerbosity"])?;
    let reject_limit = match get(cx, options, "rejectLimit")? {
        None => None,
        Some(value) => {
            let limit = integer(cx, value, "rejectLimit")?;
            match u32::try_from(limit).ok().and_then(NonZeroU32::new) {
                Some(limit) => Some(limit),
                None => return refuse(cx, "rejectLimit is a whole number from 1 to 4294967295"),
            }
        }
    };
    let log_verbosity = match get(cx, options, "logVerbosity")? {
        None => None,
        Some(value) => Some(pick(
            cx,
            value,
            "logVerbosity",
            &[
                ("verbose", CopyLogVerbosity::Verbose),
                ("silent", CopyLogVerbosity::Silent),
            ],
        )?),
    };
    let ignore = CopyIgnore {
        reject_limit,
        log_verbosity,
    };
    apply!(
        cx,
        copy,
        "ON_ERROR",
        DELIMITED_FROM,
        [CopyFrom::Text, CopyFrom::Csv],
        |s| { s.on_error_ignore(ignore) }
    )
}

fn copy_quote(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let quote = character(cx, 1, "a quote")?;
    apply!(cx, copy, "QUOTE", CSV, [CopyFrom::Csv, CopyTo::Csv], |s| s
        .quote(quote))
}

fn copy_escape(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let escape = character(cx, 1, "an escape")?;
    apply!(cx, copy, "ESCAPE", CSV, [CopyFrom::Csv, CopyTo::Csv], |s| s
        .escape(escape))
}

fn columns_at(cx: &mut FunctionContext, what: &str) -> NeonResult<Vec<pgorm::pgorm_query::Name>> {
    let value = arg(cx, 1);
    names(cx, value, what)
}

fn copy_force_not_null(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let columns = columns_at(cx, "FORCE_NOT_NULL's columns")?;
    apply!(cx, copy, "FORCE_NOT_NULL", CSV_FROM, [CopyFrom::Csv], |s| {
        s.force_not_null(columns)
    })
}

fn copy_force_not_null_all(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    apply!(cx, copy, "FORCE_NOT_NULL", CSV_FROM, [CopyFrom::Csv], |s| {
        s.force_not_null_all()
    })
}

fn copy_force_null(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let columns = columns_at(cx, "FORCE_NULL's columns")?;
    apply!(cx, copy, "FORCE_NULL", CSV_FROM, [CopyFrom::Csv], |s| s
        .force_null(columns))
}

fn copy_force_null_all(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    apply!(cx, copy, "FORCE_NULL", CSV_FROM, [CopyFrom::Csv], |s| s
        .force_null_all())
}

fn copy_force_quote(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    let columns = columns_at(cx, "FORCE_QUOTE's columns")?;
    apply!(cx, copy, "FORCE_QUOTE", CSV_TO, [CopyTo::Csv], |s| s
        .force_quote(columns))
}

fn copy_force_quote_all(cx: &mut FunctionContext) -> NeonResult<Node> {
    let copy = receiver(cx)?;
    apply!(cx, copy, "FORCE_QUOTE", CSV_TO, [CopyTo::Csv], |s| s
        .force_quote_all())
}
