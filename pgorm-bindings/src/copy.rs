//! The columns of a binary COPY a binding's program frames itself.
//!
//! A COPY's own columns come described by the server (`copy_in_columns`,
//! `copy_out_columns`), each with the type it has, an enum's or a domain's
//! included. A program writing a COPY file it never sends, or reading one
//! made elsewhere, names its columns' types instead, and a type it can name
//! without a server is one PostgreSQL itself defines.

use std::{collections::HashMap, sync::OnceLock};

use pgorm::{CopyColumn, types::Type};

use crate::{Dialect, Refusal};

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
