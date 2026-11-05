//! A value cast to a range type a schema created, in a child module of
//! `expr` beside the other cast-shaped families.

use super::*;
use crate::value_range_text::{multirange_text, range_text};

impl Expr {
    /// Cast to a range or multirange type a schema created, named in full: a
    /// range value is written as its text form first, because PostgreSQL has
    /// no cast between two range types (`42846`) and the text form is what
    /// every range type reads.
    ///
    /// A [`Value::Range`] or [`Value::Multirange`] operand becomes the text
    /// `[1,5)` (`NULL` staying `NULL`), so a built-in `int4range` value
    /// lands in a range type created over `int4`; any other operand — the
    /// text a [`Range`](crate::Range)'s `Display` writes, a column — is cast
    /// as it is. A text operand is a bound parameter pinned to `text`, or an
    /// escaped string literal inline, so the cast reads it through the range
    /// type's input function and the two renderings agree.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let slot = TypeName::new(Name::runtime("slot")).schema(Name::runtime("booking"));
    /// let query = Query::insert()
    ///     .into_table(Char::Table)
    ///     .columns([Char::Character])
    ///     .values_panic([Expr::val(Range::from(1..5)).as_range(slot)])
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"INSERT INTO "character" ("character") VALUES (CAST('[1,5)' AS booking.slot))"#
    /// );
    /// assert_eq!(
    ///     query.build().0,
    ///     r#"INSERT INTO "character" ("character") VALUES (CAST($1::text AS booking.slot))"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.value.created-range+9]
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    pub fn as_range(self, type_name: TypeName) -> SimpleExpr {
        let operand = match SimpleExpr::from(self) {
            SimpleExpr::Value(value) => SimpleExpr::Value(range_as_text(value)),
            operand => operand,
        };
        SimpleExpr::AsEnum(Box::new(type_name), Box::new(operand))
    }
}

/// `value` as the text a range type a schema created reads: a range or a
/// multirange as its text form, an array of them as the array of their
/// texts, which casts to the type's array as one text casts to the type,
/// and anything else as it is.
// [spec:pgorm:def:sql.value.created-range+9]
fn range_as_text(value: Value) -> Value {
    match value {
        Value::Range(_, range) => Value::String(range.map(|range| Box::new(range_text(&range)))),
        Value::Multirange(_, multirange) => {
            Value::String(multirange.map(|multirange| Box::new(multirange_text(&multirange))))
        }
        Value::Array(ArrayType::Range(_) | ArrayType::Multirange(_), items) => Value::Array(
            ArrayType::String,
            items.map(|items| Box::new(items.into_iter().map(range_as_text).collect())),
        ),
        value => value,
    }
}
