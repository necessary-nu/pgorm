//! Column-type introspection and [`ColumnDef`] construction.

use super::*;
use pgorm_query::{ColumnRef, GeneratedKind, IdentityGeneration, SelectExpr};

/// pgorm's utility methods that act on [ColumnType]
pub trait ColumnTypeTrait {
    /// Instantiate a new [ColumnDef]
    fn def(self) -> ColumnDef;

    /// Get the name of the enum if this is a enum column
    fn get_enum_name(&self) -> Option<&Name>;
}

// [spec:pgorm:req:entity.traits.column-def+2]
impl ColumnTypeTrait for ColumnType {
    fn def(self) -> ColumnDef {
        ColumnDef {
            col_type: self,
            null: false,
            unique: false,
            indexed: false,
            default: None,
            comment: None,
        }
    }

    fn get_enum_name(&self) -> Option<&Name> {
        enum_name(self)
    }
}

impl ColumnTypeTrait for ColumnDef {
    fn def(self) -> ColumnDef {
        self
    }

    fn get_enum_name(&self) -> Option<&Name> {
        enum_name(&self.col_type)
    }
}

fn enum_name(col_type: &ColumnType) -> Option<&Name> {
    match col_type {
        ColumnType::Enum { name, .. } => Some(name),
        ColumnType::Array(col_type) => enum_name(col_type),
        _ => None,
    }
}

/// The `LIKE`-metacharacter escape behind the substring sugar: `%`, `_` and
/// the escape character itself become literal, so search text matches
/// itself and nothing else.
// [spec:pgorm:def:entity.traits.column+7]
pub(crate) fn escape_like_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// A type's name and schema as the structured [`TypeName`] a cast renders.
fn qualified(name: &Name, schema: &Option<Name>) -> pgorm_query::TypeName {
    let type_name = pgorm_query::TypeName::new(Name::clone(name));
    match schema {
        Some(schema) => type_name.schema(Name::clone(schema)),
        None => type_name,
    }
}

/// The enum type a column stores, as the structured [`TypeName`] every cast
/// renders — schema qualification carried, the array flag set for a column
/// holding an array of the enum.
///
/// A domain over an enum is cast to by its own name, which is the type a
/// value written to its column becomes; over an enum array the domain is the
/// array type, so its name carries no `[]`. A comparison takes
/// [`compared_enum`] instead.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn enum_type_name(col_type: &ColumnType) -> Option<pgorm_query::TypeName> {
    match col_type {
        ColumnType::Enum { name, schema, .. } => Some(qualified(name, schema)),
        ColumnType::Array(col_type) => Some(enum_type_name(col_type)?.array()),
        ColumnType::Domain { name, schema, base } => {
            enum_type_name(base).map(|_| qualified(name, schema))
        }
        _ => None,
    }
}

/// The enum a column typed with a domain over one compares as, through any
/// further domains between them.
///
/// PostgreSQL resolves no operator for a domain over an enum, not even
/// between two values of the domain (`42883`, *operator does not exist: d =
/// d*), so the column and every value compared with it are cast to the enum.
/// A domain over an array of an enum is `None`: array comparison resolves for
/// it, as it does for every other domain's base.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn compared_enum(col_type: &ColumnType) -> Option<pgorm_query::TypeName> {
    let ColumnType::Domain { base, .. } = col_type else {
        return None;
    };
    match under_domains(base) {
        ColumnType::Enum { name, schema, .. } => Some(qualified(name, schema)),
        _ => None,
    }
}

/// A column as a comparison operand: cast to the enum when its domain is
/// over one ([`compared_enum`]), itself otherwise.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn compared_column<C: ColumnTrait>(col: &C, expr: Expr) -> Expr {
    match compared_enum(col.def().get_column_type()) {
        Some(enum_type) => Expr::expr(expr.cast_as_type(enum_type)),
        None => expr,
    }
}

/// A value compared with a column: cast to the enum when the column's domain
/// is over one, so it meets the column cast the same way, and through the
/// column's [`save_as`](ColumnTrait::save_as) otherwise.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn compared_value<C: ColumnTrait>(col: &C, val: Expr) -> SimpleExpr {
    compared_value_as(col.def().get_column_type(), val, |val| col.save_as(val))
}

/// [`compared_value`] for a column of `col_type` whose values `save` writes:
/// what a column described only at run time, a disjoint shape's, compares
/// by.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn compared_value_as(
    col_type: &ColumnType,
    val: Expr,
    save: impl FnOnce(Expr) -> SimpleExpr,
) -> SimpleExpr {
    match compared_enum(col_type) {
        Some(enum_type) => val.cast_as_type(enum_type),
        None => save(val),
    }
}

/// The array counterpart of [`compared_value`], for the one-array operand of
/// `= ANY` and `<> ALL`: the enum's array, or the column's
/// [`save_array_as`](ColumnTrait::save_array_as).
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn compared_array<C: ColumnTrait>(col: &C, val: Expr) -> SimpleExpr {
    match compared_enum(col.def().get_column_type()) {
        Some(enum_type) => val.cast_as_type(enum_type.array()),
        None => col.save_array_as(val),
    }
}

/// A subquery a column's membership is tested against, in the form the
/// comparison takes it.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) enum ComparedSubquery {
    /// Compared row by row, `IN (..)` / `NOT IN (..)`: as written, or with its
    /// one projected expression cast to the enum in place.
    Rows(Box<SelectStatement>),
    /// Compared as one array value, `= ANY(..)` / `<> ALL(..)`: the
    /// subquery's `ARRAY(..)` cast to the enum's array.
    Array(SimpleExpr),
}

/// A subquery a column's membership is tested against: when the column's
/// domain is over an enum, its one projected expression cast to the enum,
/// under the alias it had.
///
/// The cast is needed even where the subquery projects a column of the same
/// domain, as the column cast to the enum and the domain resolve no operator
/// either (`42883`, *operator does not exist: e = d*). A projection the cast
/// cannot reach in place — `*`, whose cast is not SQL, or a window function's,
/// which pgorm-query pairs with its `OVER` — is collected into an array
/// instead, and the array cast. The server still refuses that subquery if it
/// yields more than one column (`42601`), as it refuses it under `IN`, so a
/// wide `*` is never narrowed to its first column. Several projected
/// expressions are left as written: the membership is of one column, so the
/// server refuses them (`42601`, *subquery has too many columns*) whatever
/// their types.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn compared_subquery<C: ColumnTrait>(
    col: &C,
    mut query: SelectStatement,
) -> ComparedSubquery {
    let Some(enum_type) = compared_enum(col.def().get_column_type()) else {
        return ComparedSubquery::Rows(Box::new(query));
    };
    let [projected] = query.selects() else {
        return ComparedSubquery::Rows(Box::new(query));
    };
    let plain = projected.window().is_none()
        && !matches!(
            projected.expr(),
            SimpleExpr::Column(ColumnRef::Asterisk | ColumnRef::TableAsterisk(_))
        );
    if !plain {
        let array = Expr::expr(Expr::array(query)).cast_as_type(enum_type.array());
        return ComparedSubquery::Array(array);
    }
    let cast = projected.expr().clone().cast_as_type(enum_type);
    let projected = match &projected.alias {
        Some(alias) => SelectExpr::new_as(cast, Name::clone(alias)),
        None => SelectExpr::new(cast),
    };
    query.clear_selects();
    query.expr(projected);
    ComparedSubquery::Rows(Box::new(query))
}

/// Whether a column holds an array, under any domains it is typed with: what
/// decides whether an enum column is read as `text` or `text[]`.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn holds_array(col_type: &ColumnType) -> bool {
    match col_type {
        ColumnType::Array(_) => true,
        ColumnType::Domain { base, .. } => holds_array(base),
        _ => false,
    }
}

/// The type a column's values are, under any domains it is typed with.
pub(crate) fn under_domains(col_type: &ColumnType) -> &ColumnType {
    match col_type {
        ColumnType::Domain { base, .. } => under_domains(base),
        _ => col_type,
    }
}

/// The range or multirange type a schema created that a column stores, as
/// the [`TypeName`] a value written to it is cast to: a domain over one is
/// cast to by the domain's name, which reads the same text.
// [spec:pgorm:def:sql.value.created-range+5]
pub(crate) fn created_range_type_name(col_type: &ColumnType) -> Option<pgorm_query::TypeName> {
    match col_type {
        ColumnType::CreatedRange { name, schema, .. }
        | ColumnType::CreatedMultirange { name, schema, .. } => Some(qualified(name, schema)),
        ColumnType::Domain { name, schema, base } => {
            created_range_type_name(base).map(|_| qualified(name, schema))
        }
        _ => None,
    }
}

/// The `money` type a column stores, as the [`TypeName`] a value written to
/// it is cast to: the array flag set for a column holding an array of it, and
/// a domain over either cast to by its own name, so its constraints apply.
///
/// On the wire a `money` is an `int8` count of the currency's minor units,
/// whose scale only the server's `lc_monetary` knows, and which `Decimal`
/// does not read. Its column is therefore read cast to `numeric`
/// ([`money_carrier`]), and a value written to it is cast to the column's
/// type from the type its placeholder is pinned to, `numeric` for a
/// `Decimal` (`sql.render.cast-param-type`), PostgreSQL converting at that
/// scale both ways. A value left `numeric` would assign but not compare
/// (`42883`, *operator does not exist: money = numeric*).
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn money_type_name(col_type: &ColumnType) -> Option<pgorm_query::TypeName> {
    match col_type {
        ColumnType::Money => Some(pgorm_query::TypeName::new(Name::runtime("money"))),
        ColumnType::Array(col_type) => Some(money_type_name(col_type)?.array()),
        ColumnType::Domain { name, schema, base } => {
            money_type_name(base).map(|_| qualified(name, schema))
        }
        _ => None,
    }
}

/// The `numeric` a `money` column is read as ([`money_type_name`]), or
/// `numeric[]` for an array of them.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(crate) fn money_carrier(array: bool) -> pgorm_query::TypeName {
    let numeric = pgorm_query::TypeName::new(Name::runtime("numeric"));
    match array {
        true => numeric.array(),
        false => numeric,
    }
}

/// What fills a column an insert leaves out: a `DEFAULT` expression, the
/// sequence of an identity column, or the expression of a generated one.
///
/// The three share one slot because PostgreSQL holds them to one: a column
/// declaring any two is refused (`42601`, "both default and identity
/// specified", "both default and generation expression specified", "both
/// identity and generation expression specified"), so a definition carrying
/// two has no table to describe.
// [spec:pgorm:req:entity.traits.column-def+2]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ColumnDefault {
    /// `DEFAULT <expr>`.
    Expr(SimpleExpr),
    /// `GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY`.
    Identity(IdentityGeneration),
    /// `GENERATED ALWAYS AS (<expr>) { STORED | VIRTUAL }`.
    Generated(SimpleExpr, GeneratedKind),
}

// [spec:pgorm:req:entity.traits.column-def+2]
impl ColumnDef {
    /// Marks the column as `UNIQUE`
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }
    /// Set column comment
    pub fn comment(mut self, v: &str) -> Self {
        self.comment = Some(v.into());
        self
    }

    /// Mark the column as nullable
    pub fn null(self) -> Self {
        self.nullable()
    }

    /// Mark the column as nullable
    pub fn nullable(mut self) -> Self {
        self.null = true;
        self
    }

    /// Set the `indexed` field  to `true`
    pub fn indexed(mut self) -> Self {
        self.indexed = true;
        self
    }

    /// Set the default value, replacing an identity or a generation expression
    pub fn default_value<T>(mut self, value: T) -> Self
    where
        T: Into<Value>,
    {
        self.default = Some(ColumnDefault::Expr(value.into().into()));
        self
    }

    /// Set the default value or expression of a column, replacing an identity
    /// or a generation expression
    pub fn default<T>(mut self, default: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.default = Some(ColumnDefault::Expr(default.into()));
        self
    }

    /// Generate the column's values `ALWAYS` — `GENERATED ALWAYS AS
    /// IDENTITY` — replacing a default or a generation expression.
    ///
    /// An insert that leaves the column out takes the sequence's next value,
    /// and one that names it is refused (`428C9`), as is an update that sets
    /// it. This is how a generated column inside a composite primary key is
    /// declared — `id` in `(tenant_id, id)` — since the serial family
    /// [`auto_increment`](crate::PrimaryKeyTrait::auto_increment) draws from
    /// belongs to a one-column key alone.
    pub fn identity(mut self) -> Self {
        self.default = Some(ColumnDefault::Identity(IdentityGeneration::Always));
        self
    }

    /// Generate the column's values `BY DEFAULT` — `GENERATED BY DEFAULT AS
    /// IDENTITY` — replacing a default or a generation expression: an insert
    /// that leaves the column out takes the sequence's next value, and one that
    /// names it keeps the value it names.
    pub fn identity_by_default(mut self) -> Self {
        self.default = Some(ColumnDefault::Identity(IdentityGeneration::ByDefault));
        self
    }

    /// Compute the column from the others in its row —
    /// `GENERATED ALWAYS AS (<expr>) { STORED | VIRTUAL }` — replacing a
    /// default or an identity.
    ///
    /// A generated column is read like any other and written by nobody: an
    /// insert or update that supplies a value for it is refused (`428C9`), as
    /// one naming a `GENERATED ALWAYS` identity column is, so a model writes
    /// it by leaving it `NotSet`. A [`Virtual`](GeneratedKind::Virtual) one
    /// cannot be `unique` or `indexed`, which PostgreSQL refuses (`0A000`) when
    /// the schema is created.
    pub fn generated<T>(mut self, expr: T, kind: GeneratedKind) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.default = Some(ColumnDefault::Generated(expr.into(), kind));
        self
    }

    /// Get [ColumnType] as reference
    pub fn get_column_type(&self) -> &ColumnType {
        &self.col_type
    }

    /// Returns true if the column is nullable
    pub fn is_null(&self) -> bool {
        self.null
    }
}

pub(crate) fn cast_enum_as<F>(expr: Expr, col_type: &ColumnType, f: F) -> SimpleExpr
where
    F: Fn(Expr, pgorm_query::TypeName, &ColumnType) -> SimpleExpr,
{
    match col_type {
        #[cfg(all(feature = "with-json", feature = "postgres-array"))]
        ColumnType::Json | ColumnType::JsonBinary => {
            use pgorm_query::ArrayType;
            use serde_json::Value as Json;

            #[allow(clippy::boxed_local)]
            fn unbox<T>(boxed: Box<T>) -> T {
                *boxed
            }

            let expr = expr.into();
            match expr {
                SimpleExpr::Value(Value::Array(ArrayType::Json, Some(json_vec))) => {
                    // flatten Array(Vec<Json>) into Json
                    let json_vec: Vec<Json> = json_vec
                        .into_iter()
                        .filter_map(|val| match val {
                            Value::Json(Some(json)) => Some(unbox(json)),
                            _ => None,
                        })
                        .collect();
                    SimpleExpr::Value(Value::Json(Some(Box::new(json_vec.into()))))
                }
                SimpleExpr::Value(Value::Array(ArrayType::Json, None)) => {
                    SimpleExpr::Value(Value::Json(None))
                }
                _ => expr,
            }
        }
        _ => match enum_type_name(col_type) {
            Some(type_name) => f(expr, type_name, col_type),
            None => expr.into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::{ColumnTrait, EntityTrait};

    // [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    every
    // value-position operand passes through `save_as` — between, if_null and
    // the array membership forms included — for the derive-generated override
    // and the enum default alike
    // [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    a
    // schema-qualified enum type reaches every rendering qualified: the value
    // cast, the array cast, the CREATE TABLE column type and CREATE TYPE
    #[test]
    #[cfg(feature = "macros")]
    fn schema_qualified_enum_renders_qualified_everywhere() {
        use crate::{QueryFilter, QueryTrait, Schema};

        mod housed {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "housed")]
            pub struct Model {
                #[pgorm(primary_key, auto_increment = false)]
                pub id: i32,
                #[pgorm(column_type = ColumnType::Enum {
                    name: Name::runtime("status"),
                    schema: Some(Name::runtime("custom")),
                    variants: vec![Name::runtime("open")],
                })]
                pub status: String,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        assert_eq!(
            housed::Entity::find()
                .filter(housed::Column::Status.eq("open"))
                .as_query()
                .to_string(),
            [
                r#"SELECT "housed"."id", CAST("housed"."status" AS text)"#,
                r#"FROM "housed" WHERE "housed"."status" = CAST('open' AS custom.status)"#,
            ]
            .join(" ")
        );
        assert_eq!(
            housed::Entity::find()
                .filter(housed::Column::Status.eq_any(["open".to_owned()]))
                .as_query()
                .to_string()
                .split_once("WHERE ")
                .expect("a WHERE clause")
                .1,
            r#""housed"."status" = ANY(CAST(ARRAY ['open'] AS custom.status[]))"#
        );

        let schema = Schema::new();
        assert_eq!(
            schema
                .create_enum_from_entity(housed::Entity)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [r#"CREATE TYPE "custom"."status" AS ENUM ('open')"#]
        );
        assert!(
            schema
                .create_table_from_entity(housed::Entity)
                .expect("the entity's table is generated")
                .to_string()
                .contains(r#""status" custom.status NOT NULL"#),
            "the column type renders qualified: {}",
            schema
                .create_table_from_entity(housed::Entity)
                .expect("the entity's table is generated")
        );
    }

    /// A table whose `mood` column's domain is over an enum through a further
    /// domain.
    #[cfg(feature = "macros")]
    mod moody {
        use crate as pgorm;
        use crate::entity::prelude::*;

        /// `custom.mood`, under `app.temper`, under `app.calm_temper`.
        fn calm_temper() -> ColumnType {
            let domain = |name: &str, base: ColumnType| ColumnType::Domain {
                name: Name::runtime(name),
                schema: Some(Name::runtime("app")),
                base: Arc::new(base),
            };
            domain(
                "calm_temper",
                domain(
                    "temper",
                    ColumnType::Enum {
                        name: Name::runtime("mood"),
                        schema: Some(Name::runtime("custom")),
                        variants: vec![Name::runtime("calm"), Name::runtime("busy")],
                    },
                ),
            )
        }

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(table_name = "moody")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub id: i32,
            #[pgorm(column_type = calm_temper())]
            pub mood: String,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }

    // [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    a column whose
    // domain is over an enum compares as the enum: every predicate form casts
    // the column and its value operands to the enum, an array operand to its
    // array, while a value written to the column is still cast to the domain
    #[test]
    #[cfg(feature = "macros")]
    fn domain_enum_column_compares_as_enum() {
        use crate::{QueryFilter, QueryTrait};
        use pgorm_query::{Expr, Query};

        use moody::Column::{Id, Mood};
        let condition = |filter: pgorm_query::SimpleExpr| {
            moody::Entity::find()
                .filter(filter)
                .as_query()
                .to_string()
                .split_once("WHERE ")
                .expect("a WHERE clause")
                .1
                .to_owned()
        };
        let column = r#"CAST("moody"."mood" AS custom.mood)"#;
        let calm = "CAST('calm' AS custom.mood)";
        let busy = "CAST('busy' AS custom.mood)";
        assert_eq!(condition(Mood.eq("calm")), format!("{column} = {calm}"));
        assert_eq!(condition(Mood.lte("busy")), format!("{column} <= {busy}"));
        assert_eq!(
            condition(Mood.between("calm", "busy")),
            format!("{column} BETWEEN {calm} AND {busy}")
        );
        assert_eq!(
            condition(Mood.is_not_in(["calm", "busy"])),
            format!("{column} NOT IN ({calm}, {busy})")
        );
        assert_eq!(
            condition(Mood.eq_any(["calm".to_owned()])),
            format!("{column} = ANY(CAST(ARRAY ['calm'] AS custom.mood[]))")
        );
        assert_eq!(
            condition(Mood.ne_all(["calm".to_owned()])),
            format!("{column} <> ALL(CAST(ARRAY ['calm'] AS custom.mood[]))")
        );
        assert_eq!(
            condition(Expr::expr(Mood.if_null("calm")).is_not_null()),
            format!("COALESCE({column}, {calm}) IS NOT NULL")
        );
        assert_eq!(condition(Mood.eq_col(Mood)), format!("{column} = {column}"));
        assert_eq!(
            condition(Mood.ne_col(Id)),
            format!(r#"{column} <> "moody"."id""#)
        );
        assert_eq!(
            condition(Mood.eq_expr(Expr::val("calm"))),
            format!("{column} = 'calm'")
        );
        assert_eq!(
            Query::insert()
                .into_table(moody::Entity)
                .columns([Mood])
                .values_panic([Mood.save_as(Expr::val("calm"))])
                .to_string(),
            r#"INSERT INTO "moody" ("mood") VALUES (CAST('calm' AS app.calm_temper))"#
        );
    }

    // [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    a column whose
    // domain is over an enum is compared as the enum in subquery membership,
    // the subquery's one plain projection cast with it, a `*`, a table's `*` or
    // a windowed projection compared through the subquery's array cast to the
    // enum's, several projections left as written, aggregated as the enum by
    // max and min but not by count,
    // and cast on its side of a relation's join, reversal included, while a
    // pair of plain columns joins bare
    // [spec:pgorm:sem:query.build.join+5/test]    each side of a pair whose
    // column's domain is over an enum is cast, and its record follows rev()
    #[test]
    #[cfg(feature = "macros")]
    fn domain_enum_subqueries_joins_and_extrema_cast() {
        use crate::{JoinType, QueryFilter, QuerySelect, QueryTrait, RelationDef};
        use pgorm_query::{Asterisk, Expr, Func, Name, Query, WindowStatement, alias};

        use moody::Column::{Id, Mood};
        let condition = |filter: pgorm_query::SimpleExpr| {
            moody::Entity::find()
                .filter(filter)
                .as_query()
                .to_string()
                .split_once("WHERE ")
                .expect("a WHERE clause")
                .1
                .to_owned()
        };
        let column = r#"CAST("moody"."mood" AS custom.mood)"#;
        let moods = || Query::select().column(Mood).from(moody::Entity).to_owned();
        assert_eq!(
            condition(Mood.in_subquery(moods())),
            format!(r#"{column} IN (SELECT CAST("mood" AS custom.mood) FROM "moody")"#)
        );
        assert_eq!(
            condition(
                Mood.not_in_subquery(
                    Query::select()
                        .expr_as(Expr::col(Mood), alias("m"))
                        .from(moody::Entity)
                        .to_owned()
                )
            ),
            format!(r#"{column} NOT IN (SELECT CAST("mood" AS custom.mood) AS "m" FROM "moody")"#)
        );
        let array = |subquery: &str| format!("CAST(ARRAY({subquery}) AS custom.mood[])");
        assert_eq!(
            condition(
                Mood.in_subquery(
                    Query::select()
                        .column(Asterisk)
                        .from(moody::Entity)
                        .to_owned()
                )
            ),
            format!("{column} = ANY({})", array(r#"SELECT * FROM "moody""#))
        );
        assert_eq!(
            condition(
                Mood.not_in_subquery(
                    Query::select()
                        .column((moody::Entity, Asterisk))
                        .from(moody::Entity)
                        .to_owned()
                )
            ),
            format!(
                "{column} <> ALL({})",
                array(r#"SELECT "moody".* FROM "moody""#)
            )
        );
        assert_eq!(
            condition(
                Mood.in_subquery(
                    Query::select()
                        .expr_window_as(
                            Func::named(Name::runtime("lag")).arg(Expr::col(Mood)),
                            WindowStatement::partition_by(Id),
                            alias("previous"),
                        )
                        .from(moody::Entity)
                        .to_owned()
                )
            ),
            format!(
                "{column} = ANY({})",
                array(
                    r#"SELECT lag("mood") OVER ( PARTITION BY "id" ) AS "previous" FROM "moody""#
                )
            )
        );
        assert_eq!(
            condition(
                Mood.in_subquery(
                    Query::select()
                        .columns([Id, Mood])
                        .from(moody::Entity)
                        .to_owned()
                )
            ),
            format!(r#"{column} IN (SELECT "id", "mood" FROM "moody")"#)
        );
        assert_eq!(
            condition(Id.in_subquery(Query::select().column(Id).from(moody::Entity).to_owned())),
            r#""moody"."id" IN (SELECT "id" FROM "moody")"#
        );

        let extrema = moody::Entity::find()
            .select_only()
            .expr(Mood.max())
            .expr(Mood.min())
            .expr(Mood.count())
            .as_query()
            .to_string();
        assert_eq!(
            extrema,
            format!(r#"SELECT MAX({column}), MIN({column}), COUNT("moody"."mood") FROM "moody""#)
        );

        let joined = |rel: RelationDef| {
            moody::Entity::find()
                .select_only()
                .column(Id)
                .join_as(JoinType::InnerJoin, rel, alias("other"))
                .as_query()
                .to_string()
                .split_once(" ON ")
                .expect("an ON clause")
                .1
                .to_owned()
        };
        let other = r#"CAST("other"."mood" AS custom.mood)"#;
        let both: RelationDef = moody::Entity::belongs_to(moody::Entity)
            .columns(Mood, Mood)
            .and_columns(Id, Id)
            .into();
        assert_eq!(
            joined(both),
            format!(r#"{column} = {other} AND "moody"."id" = "other"."id""#)
        );
        let crossed = || -> RelationDef {
            moody::Entity::belongs_to(moody::Entity)
                .columns(Mood, Id)
                .into()
        };
        assert_eq!(joined(crossed()), format!(r#"{column} = "other"."id""#));
        assert_eq!(
            joined(crossed().rev()),
            format!(r#""moody"."id" = {other}"#)
        );
    }
}
