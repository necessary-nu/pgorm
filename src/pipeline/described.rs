//! Relations a program describes at run time, with no entity behind them:
//! a column typed by a [`ColumnType`] the program names, and the sources'
//! projection [`select_sources`](Pipeline::select_sources) writes for
//! entities, written for sources the program lists column by column.
//!
//! A language binding declares its models as values, so it has a table, its
//! columns' names and their types, and no Rust type to hand `Named::col` or
//! `select_sources`. These are the same writers, keyed by what the binding
//! has instead.

use pgorm_query::{ColumnType, IntoName, SqlName, TypeName};

use crate::entity::{compared_enum, compared_type_cast};
use crate::executor::result_name::result_column_name;
use crate::query::graph::source_prefix;

use super::adapter;
use super::builder::Pipeline;
use super::error::PipelineError;
use super::expr::{Expr, col};
use super::sources::{appended_projection, typed_cast};

/// A column of `table` no entity describes, compared as an entity column of
/// `column_type` is: a column whose domain is over an enum cast to the enum,
/// and a literal or bound value compared with it cast as the type's own write
/// cast casts one — an enum's label to the enum, a `money` value to `money`,
/// a created range's text to its type. [`col`] only qualifies; this also
/// types.
///
/// ```
/// use pgorm::pgorm_query::{ColumnType, Name};
/// use pgorm::pipeline::{ExprOps, Pipeline, alias, col_typed};
///
/// let flavor = ColumnType::Enum {
///     name: Name::runtime("flavor"),
///     schema: None,
///     variants: vec![Name::runtime("lemon")],
/// };
/// let (sql, _) = Pipeline::from(alias("cake"))
///     .filter(col_typed(alias("cake"), alias("flavor"), &flavor).eq("lemon"))
///     .into_sql()?;
/// assert_eq!(sql, "SELECT * FROM cake WHERE flavor = CAST('lemon' AS flavor)");
/// # Ok::<_, pgorm::pipeline::PipelineError>(())
/// ```
// [spec:pgorm:sem:pipeline.described]
// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
pub fn col_typed<'brand>(
    table: impl IntoName,
    column: impl IntoName,
    column_type: &ColumnType,
) -> Expr<'brand> {
    let mut expr = col(table, column);
    expr.compares_as = compared_enum(column_type);
    expr.values_as = compared_type_cast(column_type);
    expr
}

/// One relation a pipeline read, restated to
/// [`select_described`](Pipeline::select_described): the name its columns are
/// qualified by — its table's, or the name it was read under — and each
/// column to project, with the type it is read as where its reader needs a
/// cast.
// [spec:pgorm:sem:pipeline.described]
#[derive(Debug, Clone)]
pub struct DescribedSource {
    qualifier: String,
    columns: Vec<(String, Option<TypeName>)>,
}

impl DescribedSource {
    /// A source of no columns yet, its columns qualified by `qualifier`.
    pub fn new(qualifier: impl IntoName) -> Self {
        Self {
            qualifier: SqlName::to_string(&*qualifier.into_name()),
            columns: Vec::new(),
        }
    }

    /// The source with `column` projected after its others, cast to
    /// `read_as` when one is given.
    pub fn column(mut self, column: impl IntoName, read_as: Option<TypeName>) -> Self {
        self.columns
            .push((SqlName::to_string(&*column.into_name()), read_as));
        self
    }
}

impl Pipeline {
    /// [`select_sources`](Pipeline::select_sources) for sources no entity
    /// describes: the pipeline with one final projection stage holding, for
    /// the i-th listed source, each of its columns in the order listed,
    /// qualified by the source's name, cast to its `read_as` type where it has
    /// one, and named `s{i}_{column}` as `select_sources` names it, bounded to
    /// 63 bytes by the same composition. Decoding the rows is the caller's,
    /// each source's columns found under its prefix.
    ///
    /// A pipeline that has replaced its sources' own columns is refused with
    /// [`PipelineError::ReshapedSources`] naming the stage, as
    /// `select_sources` refuses it, and the projection is such a stage itself,
    /// so the pipeline returned refuses a second selection.
    ///
    /// ```
    /// use pgorm::pipeline::{DescribedSource, ExprOps, JoinSide, Pipeline, alias, col};
    ///
    /// let (cake, bakery) = (alias("cake"), alias("bakery"));
    /// let (sql, _) = Pipeline::from(cake)
    ///     .join(JoinSide::Inner, bakery, col(cake, alias("bakery_id")).eq(col(bakery, alias("id"))))
    ///     .select_described([
    ///         DescribedSource::new(cake).column(alias("id"), None),
    ///         DescribedSource::new(bakery).column(alias("name"), None),
    ///     ])?
    ///     .into_sql()?;
    /// assert_eq!(
    ///     sql,
    ///     "SELECT cake.id AS s0_id, bakery.name AS s1_name FROM cake \
    ///      INNER JOIN bakery ON cake.bakery_id = bakery.id"
    /// );
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    // [spec:pgorm:sem:pipeline.described]
    pub fn select_described(
        self,
        sources: impl IntoIterator<Item = DescribedSource>,
    ) -> Result<Pipeline, PipelineError> {
        let mut projection = Vec::new();
        let mut count = 0;
        for (index, source) in sources.into_iter().enumerate() {
            count = index + 1;
            let prefix = source_prefix(index);
            for (column, read_as) in source.columns {
                let alias = result_column_name(&prefix, &column);
                let node = adapter::ident_in(vec![source.qualifier.clone()], column);
                let node = match &read_as {
                    Some(cast) => typed_cast(node, cast),
                    None => node,
                };
                projection.push(adapter::aliased(node, alias));
            }
        }
        let mut pipeline = appended_projection(self, projection, count)?;
        pipeline.reshaped = Some("select");
        Ok(pipeline)
    }
}

#[cfg(test)]
mod tests {
    use pgorm_query::{ColumnType, Name, TypeName, Value};

    use super::{DescribedSource, col_typed};
    use crate::pipeline::{ExprOps, Pipeline, PipelineError, alias, col};

    fn flavor() -> ColumnType {
        ColumnType::Enum {
            name: Name::runtime("flavor"),
            schema: Some(Name::runtime("public")),
            variants: vec![Name::runtime("lemon"), Name::runtime("carrot")],
        }
    }

    // [spec:pgorm:sem:pipeline.described/test]
    #[test]
    fn typed_column_casts_compared_values() {
        let cake = alias("cake");
        let (sql, values) = Pipeline::from(cake)
            .filter_with(|binder| {
                col_typed(cake, alias("flavor"), &flavor()).eq(binder.bind("lemon"))
            })
            .filter(col_typed(cake, alias("flavor"), &flavor()).in_array(["lemon", "carrot"]))
            .filter(col_typed(cake, alias("price"), &ColumnType::Decimal(None)).gt(4))
            .into_sql()
            .expect("the pipeline compiles");
        assert_eq!(
            sql,
            "SELECT * FROM cake WHERE flavor = CAST($1::text AS public.flavor) AND flavor IN \
             (CAST('lemon' AS public.flavor), CAST('carrot' AS public.flavor)) AND price > 4"
        );
        assert_eq!(values.0, [Value::from("lemon")]);
    }

    // [spec:pgorm:sem:pipeline.described/test]
    #[test]
    fn typed_domain_column_compares_as_its_enum() {
        let mood = ColumnType::Domain {
            name: Name::runtime("cheerful"),
            schema: None,
            base: std::sync::Arc::new(ColumnType::Enum {
                name: Name::runtime("mood"),
                schema: None,
                variants: vec![Name::runtime("glad")],
            }),
        };
        let line = alias("line");
        let (sql, _) = Pipeline::from(line)
            .filter(col_typed(line, alias("mood"), &mood).eq("glad"))
            .into_sql()
            .expect("the pipeline compiles");
        assert_eq!(
            sql,
            "SELECT * FROM line WHERE CAST(mood AS mood) = CAST('glad' AS mood)"
        );
    }

    // [spec:pgorm:sem:pipeline.described/test]
    #[test]
    fn typed_created_range_binds_its_text() {
        let span = ColumnType::CreatedRange {
            name: Name::runtime("floatrange"),
            schema: Some(Name::runtime("public")),
            subtype: std::sync::Arc::new(ColumnType::Double),
        };
        let shift = alias("shift");
        let (sql, _) = Pipeline::from(shift)
            .filter_with(|binder| col_typed(shift, alias("span"), &span).eq(binder.bind("[1,2)")))
            .into_sql()
            .expect("the pipeline compiles");
        assert_eq!(
            sql,
            "SELECT * FROM shift WHERE span = CAST($1::text AS public.floatrange)"
        );
    }

    // [spec:pgorm:sem:pipeline.described/test]
    #[test]
    fn described_sources_take_prefixes_and_read_casts() {
        let (cake, other) = (alias("cake"), alias("other"));
        let text = TypeName::new(alias("text"));
        let (sql, _) = Pipeline::from(cake)
            .join(
                crate::pipeline::JoinSide::Left,
                crate::pipeline::IntoSource::named(cake, other),
                col(cake, alias("id")).lt(col(other, alias("id"))),
            )
            .select_described([
                DescribedSource::new(cake)
                    .column(alias("id"), None)
                    .column(alias("spans"), Some(text.clone())),
                DescribedSource::new(other)
                    .column(alias("id"), None)
                    .column(alias("spans"), Some(text.array())),
            ])
            .expect("nothing reshaped the sources")
            .into_sql()
            .expect("the pipeline compiles");
        assert_eq!(
            sql,
            "SELECT cake.id AS s0_id, CAST(cake.spans AS text) AS s0_spans, other.id AS s1_id, \
             CAST(other.spans AS text[]) AS s1_spans FROM cake LEFT OUTER JOIN cake AS other \
             ON cake.id < other.id"
        );
    }

    // [spec:pgorm:sem:pipeline.described/test]
    #[test]
    fn described_sources_refuse_reshaped_pipelines() {
        let cake = alias("cake");
        let source = || DescribedSource::new(cake).column(alias("id"), None);
        assert_eq!(
            Pipeline::from(cake)
                .select(col(cake, alias("id")))
                .select_described([source()])
                .err(),
            Some(PipelineError::ReshapedSources("select"))
        );
        let selected = Pipeline::from(cake)
            .select_described([source()])
            .expect("nothing reshaped the sources");
        assert_eq!(
            selected.select_described([source()]).err(),
            Some(PipelineError::ReshapedSources("select"))
        );
    }

    // [spec:pgorm:sem:pipeline.described/test]
    #[test]
    fn described_column_names_are_bounded() {
        let long = "a".repeat(61);
        let table = alias("t");
        let (sql, _) = Pipeline::from(table)
            .select_described([DescribedSource::new(table).column(Name::runtime(&long), None)])
            .expect("nothing reshaped the sources")
            .into_sql()
            .expect("the pipeline compiles");
        let alias_name = crate::executor::result_name::result_column_name("s0_", &long);
        assert!(alias_name.len() <= 63);
        assert_eq!(sql, format!("SELECT {long} AS {alias_name} FROM t"));
    }
}
