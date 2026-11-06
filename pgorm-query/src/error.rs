//! Error types used in pgorm-query.

/// Result type for pgorm-query
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Column and value vector having different length
    ColValNumMismatch { col_len: usize, val_len: usize },
    /// A placeholder template could not be paired with the substitutions
    /// supplied for it.
    Template {
        /// The template as written.
        template: String,
        /// What about the pairing was rejected.
        reason: TemplateError,
    },
    /// A `FETCH FIRST .. WITH TIES` on a statement with no ORDER BY: its ties
    /// are measured on the order, and PostgreSQL's grammar refuses one
    /// without it (`42601`).
    TiesWithoutOrder,
    /// A `FETCH FIRST .. WITH TIES` beside `SKIP LOCKED`, which PostgreSQL's
    /// grammar refuses (`42601`): skipping locked rows would make the tied
    /// set depend on what other transactions hold.
    TiesWithSkipLocked,
    /// A write converted into a COPY TO's source without a `RETURNING`
    /// list. COPY TO copies the rows a write returns, and PostgreSQL refuses
    /// a write that returns none (`0A000`).
    CopyWithoutReturning {
        /// The write's keyword: `INSERT`, `UPDATE`, `DELETE` or `MERGE`.
        write: &'static str,
    },
    /// A WINDOW clause naming two windows alike, which PostgreSQL refuses
    /// (`42P20`).
    WindowRedefined {
        /// The name the entries share.
        name: String,
    },
    /// A projection over a window name the statement's WINDOW clause does
    /// not declare (`42704`).
    WindowUndefined {
        /// The name the projection reads.
        name: String,
    },
    /// A window extending a base that is no earlier entry of its WINDOW
    /// clause, which PostgreSQL reads as no window at all (`42704`).
    WindowBaseUndefined {
        /// The extending window's name in the WINDOW clause, or `None` for
        /// one written inline after `OVER`.
        window: Option<String>,
        /// The base it names.
        base: String,
    },
    /// A window extending a base that has a frame, which PostgreSQL refuses
    /// to copy (`42P20`).
    WindowFrameCopied {
        /// The extending window's name in the WINDOW clause, or `None` for
        /// one written inline after `OVER`.
        window: Option<String>,
        /// The base it names.
        base: String,
    },
    /// A window adding an ORDER BY to a base that already orders its rows,
    /// by its own ORDER BY or one it copied, which PostgreSQL refuses to
    /// override (`42P20`).
    WindowOrderOverridden {
        /// The extending window's name in the WINDOW clause, or `None` for
        /// one written inline after `OVER`.
        window: Option<String>,
        /// The base it names.
        base: String,
    },
}

/// Why a placeholder template and its substitutions do not pair up.
///
/// A template's placeholder census is fully known the moment the template
/// meets its values, so every one of these is reported when the pair is built
/// — see [`SqlTemplate::new`](crate::SqlTemplate::new) and
/// [`inject_parameters`](crate::inject_parameters).
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateError {
    /// A `$` is followed by neither `$` nor a placeholder index.
    MalformedPlaceholder {
        /// Character offset of the offending `$` within the template.
        position: usize,
    },
    /// The template references `$0`; placeholders are numbered from 1.
    ZeroIndex,
    /// The template references a placeholder beyond the substitutions supplied.
    IndexOutOfRange {
        /// The index referenced.
        index: usize,
        /// How many substitutions were supplied.
        supplied: usize,
    },
    /// A supplied substitution is never referenced by the template.
    UnreferencedValue {
        /// The 1-based index of the substitution left unused.
        index: usize,
        /// How many substitutions were supplied.
        supplied: usize,
    },
    /// A piece of a template literal writes a `$N` marker, where its values
    /// are the ones written between its pieces
    /// ([`SqlTemplate::from_pieces`](crate::SqlTemplate::from_pieces)).
    MarkerInText {
        /// The index the marker names.
        index: usize,
    },
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::ColValNumMismatch { col_len, val_len } => write!(
                f,
                "Columns and values length mismatch: {col_len} != {val_len}"
            ),
            Self::Template { template, reason } => {
                write!(f, "Invalid placeholder template \"{template}\": {reason}")
            }
            Self::TiesWithoutOrder => {
                f.write_str("FETCH FIRST .. WITH TIES needs an ORDER BY to measure its ties on")
            }
            Self::TiesWithSkipLocked => {
                f.write_str("FETCH FIRST .. WITH TIES cannot be combined with SKIP LOCKED")
            }
            Self::CopyWithoutReturning { write } => write!(
                f,
                "COPY TO copies the rows a write returns, and this {write} has no RETURNING list"
            ),
            Self::WindowRedefined { name } => {
                write!(f, "the WINDOW clause names two windows \"{name}\"")
            }
            Self::WindowUndefined { name } => {
                write!(f, "no window \"{name}\" is declared in the WINDOW clause")
            }
            Self::WindowBaseUndefined {
                window: Some(window),
                base,
            } => write!(
                f,
                "window \"{window}\" extends \"{base}\", which no earlier entry of the \
                 WINDOW clause declares"
            ),
            Self::WindowBaseUndefined { window: None, base } => write!(
                f,
                "the window after OVER extends \"{base}\", which the WINDOW clause does \
                 not declare"
            ),
            Self::WindowFrameCopied { window, base } => write!(
                f,
                "{} cannot extend \"{base}\", which has a frame",
                WindowSubject(window)
            ),
            Self::WindowOrderOverridden { window, base } => write!(
                f,
                "{} cannot add an ORDER BY to \"{base}\", which already orders its rows",
                WindowSubject(window)
            ),
        }
    }
}

/// How a window refusal names the window it refuses: by its name in the
/// WINDOW clause, or as the one written inline after `OVER`.
#[derive(Debug)]
struct WindowSubject<'a>(&'a Option<String>);

impl std::fmt::Display for WindowSubject<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.0 {
            Some(name) => write!(f, "window \"{name}\""),
            None => f.write_str("the window after OVER"),
        }
    }
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::MalformedPlaceholder { position } => write!(
                f,
                "the `$` at character {position} is not a placeholder; write `$$` for a literal `$`"
            ),
            Self::ZeroIndex => {
                write!(f, "placeholders are numbered from 1, so `$0` names nothing")
            }
            Self::IndexOutOfRange { index, supplied } => write!(
                f,
                "`${index}` is referenced but only {supplied} substitution(s) were supplied"
            ),
            Self::UnreferencedValue { index, supplied } => write!(
                f,
                "{supplied} substitution(s) were supplied but `${index}` is never referenced"
            ),
            Self::MarkerInText { index } => write!(
                f,
                "`${index}` is written in the template's text; its values are the ones written between its pieces"
            ),
        }
    }
}
