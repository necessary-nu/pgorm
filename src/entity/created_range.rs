use pgorm_query::{Expr, SimpleExpr, TypeName, Value};

/// A Rust type standing for a range type a schema created with `CREATE TYPE
/// ... AS RANGE`, or for the multirange PostgreSQL creates beside it: a
/// newtype over [`Range<T>`](pgorm_query::Range) or
/// [`Multirange<T>`](pgorm_query::Multirange), named the way an
/// [`ActiveEnum`](crate::ActiveEnum) names its enum type, because the type's
/// name is what PostgreSQL needs to read a value of it and no [`Value`] can
/// carry one.
///
/// A value converts into its text form, `[1.5,2.5)`, as
/// [`Value::String`]: each bound is written in its subtype's text and
/// quoted where the range parser needs it. A column of the type is
/// [`ColumnType::CreatedRange`](crate::ColumnType::CreatedRange), whose
/// [`save_as`](crate::ColumnTrait::save_as) casts that text to the type by
/// name, bound as a `text` parameter or escaped inline; there is no cast
/// between two range types, so a built-in range value written to the column
/// is turned into its text first. A row of a range decodes from the range's
/// binary form, over whatever subtype the server reports; a row of a
/// multirange, which the driver reports as a simple type, is read as its
/// text, the column's [`select_as`](crate::ColumnTrait::select_as) casting it
/// so.
///
/// Derived with [`CreatedRange`](pgorm_macros::CreatedRange):
///
/// ```
/// use pgorm::entity::prelude::*;
///
/// #[derive(Clone, Debug, PartialEq, CreatedRange)]
/// #[pgorm(range_name = "floatrange", schema_name = "measure")]
/// pub struct FloatRange(pub Range<f64>);
///
/// assert_eq!(
///     FloatRange::from(Range::from(1.5..2.5)).into_expr(),
///     Expr::val("[1.5,2.5)").cast_as_type(
///         pgorm::pgorm_query::TypeName::new(Name::runtime("floatrange"))
///             .schema(Name::runtime("measure")),
///     ),
/// );
/// ```
// [spec:pgorm:def:sql.value.created-range+9]
pub trait CreatedRange: Into<Value> {
    /// The range type's name, schema-qualified when it was declared with one.
    fn name() -> TypeName;

    /// This value as an expression of its range type: its text form cast to
    /// the type.
    fn into_expr(self) -> SimpleExpr {
        Expr::val(self).as_range(Self::name())
    }
}
