//! The columns of a binary COPY a binding's program frames itself.
//!
//! A COPY's own columns come described by the server (`copy_in_columns`,
//! `copy_out_columns`), each with the type it has, an enum's or a domain's
//! included. A program writing a COPY file it never sends, or reading one
//! made elsewhere, names its columns' types instead, and a type it can name
//! without a server is one PostgreSQL itself defines.

use std::{collections::HashMap, sync::OnceLock};

use pgorm::{
    CopyColumn, CopyInSink, CopyOutStream, CopyTrait, Error,
    pgorm_query::{
        CopyBinary, CopyCsv, CopyFromStatement, CopyText, CopyToStatement, IntoCopySource,
        TableName,
    },
    types::Type,
};

use crate::{Dialect, Refusal};

/// A COPY's data format, which decides the options it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    /// PostgreSQL's text format: tab-separated, `\N` for null.
    Text,
    /// CSV, which takes its quoting options.
    Csv,
    /// PostgreSQL's binary format, the one a COPY of rows uses.
    Binary,
}

/// A `COPY … FROM STDIN` in one of its three formats, as a binding's
/// builder holds it.
// [spec:pgorm:req:bindings.copy]
#[derive(Debug, Clone)]
pub enum CopyFrom {
    /// In the text format.
    Text(CopyFromStatement<CopyText>),
    /// In CSV.
    Csv(CopyFromStatement<CopyCsv>),
    /// In the binary format.
    Binary(CopyFromStatement<CopyBinary>),
}

/// A `COPY … TO STDOUT` in one of its three formats.
// [spec:pgorm:req:bindings.copy]
#[derive(Debug, Clone)]
pub enum CopyTo {
    /// In the text format.
    Text(CopyToStatement<CopyText>),
    /// In CSV.
    Csv(CopyToStatement<CopyCsv>),
    /// In the binary format.
    Binary(CopyToStatement<CopyBinary>),
}

impl CopyFrom {
    /// `COPY table FROM STDIN` in `format`.
    pub fn new(table: TableName, format: DataFormat) -> Self {
        match format {
            DataFormat::Text => Self::Text(CopyFromStatement::new(table, CopyText)),
            DataFormat::Csv => Self::Csv(CopyFromStatement::new(table, CopyCsv)),
            DataFormat::Binary => Self::Binary(CopyFromStatement::new(table, CopyBinary)),
        }
    }

    /// The format's name in a COPY's own options.
    pub fn format(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Csv(_) => "csv",
            Self::Binary(_) => "binary",
        }
    }

    /// pgorm-query's one rendering of the statement, every value in it an
    /// escaped literal.
    pub fn rendered(&self) -> String {
        match self {
            Self::Text(statement) => statement.to_string(),
            Self::Csv(statement) => statement.to_string(),
            Self::Binary(statement) => statement.to_string(),
        }
    }

    /// The columns the COPY writes, as pgorm describes them before it
    /// starts, starting none.
    // [spec:pgorm:req:bindings.copy-rows+2]
    pub async fn columns<C: CopyTrait + Send>(&self, db: &mut C) -> Result<Vec<CopyColumn>, Error> {
        match self {
            Self::Text(statement) => db.copy_in_columns(statement).await,
            Self::Csv(statement) => db.copy_in_columns(statement).await,
            Self::Binary(statement) => db.copy_in_columns(statement).await,
        }
    }

    /// The COPY started on `db`, its sink borrowing the connection.
    pub async fn open<'a, C: CopyTrait + Send>(
        &self,
        db: &'a mut C,
    ) -> Result<CopyInSink<'a>, Error> {
        match self {
            Self::Text(statement) => db.copy_in(statement).await,
            Self::Csv(statement) => db.copy_in(statement).await,
            Self::Binary(statement) => db.copy_in(statement).await,
        }
    }
}

impl CopyTo {
    /// `COPY source TO STDOUT` in `format`.
    pub fn new(source: impl IntoCopySource, format: DataFormat) -> Self {
        let source = source.into_copy_source();
        match format {
            DataFormat::Text => Self::Text(CopyToStatement::new(source, CopyText)),
            DataFormat::Csv => Self::Csv(CopyToStatement::new(source, CopyCsv)),
            DataFormat::Binary => Self::Binary(CopyToStatement::new(source, CopyBinary)),
        }
    }

    /// The format's name in a COPY's own options.
    pub fn format(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Csv(_) => "csv",
            Self::Binary(_) => "binary",
        }
    }

    /// pgorm-query's one rendering of the statement, every value in it an
    /// escaped literal.
    pub fn rendered(&self) -> String {
        match self {
            Self::Text(statement) => statement.to_string(),
            Self::Csv(statement) => statement.to_string(),
            Self::Binary(statement) => statement.to_string(),
        }
    }

    /// The columns the COPY reads, as pgorm describes them before it starts,
    /// starting none.
    // [spec:pgorm:req:bindings.copy-rows+2]
    pub async fn columns<C: CopyTrait + Send>(&self, db: &mut C) -> Result<Vec<CopyColumn>, Error> {
        match self {
            Self::Text(statement) => db.copy_out_columns(statement).await,
            Self::Csv(statement) => db.copy_out_columns(statement).await,
            Self::Binary(statement) => db.copy_out_columns(statement).await,
        }
    }

    /// The COPY started on `db`, its stream borrowing the connection.
    pub async fn open<'a, C: CopyTrait + Send>(
        &self,
        db: &'a mut C,
    ) -> Result<CopyOutStream<'a>, Error> {
        match self {
            Self::Text(statement) => db.copy_out(statement).await,
            Self::Csv(statement) => db.copy_out(statement).await,
            Self::Binary(statement) => db.copy_out(statement).await,
        }
    }
}

/// The first OID PostgreSQL gives an object it did not create itself, so
/// every built-in type's is below it.
const FIRST_NORMAL_OBJECT_ID: u32 = 16_384;

/// Every built-in type tokio-postgres knows, by the name `pg_type` gives it:
/// `int4`, `varchar`, `_text` for an array of `text`.
fn builtins() -> &'static HashMap<String, Type> {
    static BUILTINS: OnceLock<HashMap<String, Type>> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        (0..FIRST_NORMAL_OBJECT_ID)
            .filter_map(Type::from_oid)
            .map(|ty| (ty.name().to_owned(), ty))
            .collect()
    })
}

/// The built-in type `name` names: its `pg_type` name, as a described
/// column's type reports it, or an element's name followed by `[]` for an
/// array of it, `text[]` being `_text`.
pub fn builtin_type(name: &str) -> Option<Type> {
    let typname = match name.strip_suffix("[]") {
        Some(element) => format!("_{element}"),
        None => name.to_owned(),
    };
    builtins().get(&typname).cloned()
}

/// A column of `type_name`, a built-in type, for a COPY a program frames
/// itself. A `money` column needs the places its currency is written to,
/// which a described column asks the server for; no other column takes
/// them.
// [spec:pgorm:req:bindings.copy-rows+2]
pub fn copy_column(
    name: String,
    type_name: &str,
    money_places: Option<u32>,
    dialect: Dialect,
) -> Result<CopyColumn, Refusal> {
    let ty = builtin_type(type_name).ok_or_else(|| {
        Refusal(format!(
            "{type_name} is no built-in PostgreSQL type; describe the COPY's columns to learn \
             another type"
        ))
    })?;
    let places = dialect.option("money_places");
    match (ty == Type::MONEY, money_places) {
        (true, None) => Err(Refusal(format!(
            "a money column needs {places}, the places its currency is written to"
        ))),
        (true, Some(money)) => Ok(CopyColumn::new(name, ty).with_money_places(money)),
        (false, Some(_)) => Err(Refusal(format!(
            "{places} is for a money column, and {name} is {type_name}"
        ))),
        (false, None) => Ok(CopyColumn::new(name, ty)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_types_are_found_by_name() {
        assert_eq!(builtin_type("int4"), Some(Type::INT4));
        assert_eq!(builtin_type("varchar"), Some(Type::VARCHAR));
        assert_eq!(builtin_type("text[]"), Some(Type::TEXT_ARRAY));
        assert_eq!(builtin_type("_text"), Some(Type::TEXT_ARRAY));
        assert_eq!(builtin_type("integer"), None);
        assert_eq!(builtin_type("mood"), None);
    }

    #[test]
    fn only_money_takes_money_places() {
        let refused = |column: Result<CopyColumn, Refusal>| match column {
            Err(refusal) => refusal.message().to_owned(),
            Ok(column) => panic!("{column:?} was not refused"),
        };
        assert_eq!(
            copy_column("price".to_owned(), "money", Some(2), Dialect::Python)
                .map(|column| column.money_places()),
            Ok(Some(2))
        );
        assert_eq!(
            refused(copy_column(
                "price".to_owned(),
                "money",
                None,
                Dialect::JavaScript
            )),
            "a money column needs moneyPlaces, the places its currency is written to"
        );
        assert_eq!(
            refused(copy_column(
                "name".to_owned(),
                "text",
                Some(2),
                Dialect::Python
            )),
            "money_places is for a money column, and name is text"
        );
        assert_eq!(
            refused(copy_column(
                "mood".to_owned(),
                "mood",
                None,
                Dialect::Python
            )),
            "mood is no built-in PostgreSQL type; describe the COPY's columns to learn another type"
        );
    }
}
