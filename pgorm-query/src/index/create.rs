use inherent::inherent;

use crate::{
    QueryBuilder,
    expr::SimpleExpr,
    query::{ConditionHolder, ConditionalStatement, IntoCondition},
    types::*,
};

use super::common::*;

/// Create an index for an existing table
///
/// An index indexes at least one column of exactly one table, so both are
/// taken by the constructor and `col` appends the remaining columns: neither
/// the empty column list nor the missing `ON` target PostgreSQL rejects has
/// anywhere to come from. The index name stays optional, because PostgreSQL
/// derives one when it is absent.
///
/// This is the standalone `CREATE INDEX` and nothing else. A unique or
/// primary key written inside `CREATE TABLE` is a
/// [`TableKey`](crate::TableKey), a type of its own, because the
/// table-constraint grammar has no place for most of what an index carries — a
/// non-unique kind, an expression or ordered entry, an operator class, a
/// predicate, an access method.
///
/// ```compile_fail,E0061
/// use pgorm_query::{*, tests_cfg::*};
///
/// Index::create(Glyph::Aspect).name(Name::runtime("idx-glyph-aspect"));
/// ```
///
/// # Examples
///
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let index = Index::create(Glyph::Table, Glyph::Aspect)
///     .name(Name::runtime("idx-glyph-aspect"))
///     .to_owned();
///
/// assert_eq!(
///     index.to_string(),
///     r#"CREATE INDEX "idx-glyph-aspect" ON "glyph" ("aspect")"#
/// );
/// ```
/// Create index if not exists
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let index = Index::create(Glyph::Table, Glyph::Aspect)
///     .if_not_exists()
///     .name(Name::runtime("idx-glyph-aspect"))
///     .to_owned();
///
/// assert_eq!(
///     index.to_string(),
///     r#"CREATE INDEX IF NOT EXISTS "idx-glyph-aspect" ON "glyph" ("aspect")"#
/// );
/// ```
/// A column takes no prefix length. That is MySQL's index-a-leading-substring
/// syntax, which PostgreSQL rejects outright, so the tuple that spelled it does
/// not typecheck:
///
/// ```compile_fail,E0277
/// use pgorm_query::{*, tests_cfg::*};
///
/// Index::create(Glyph::Table, (Glyph::Aspect, 128));
/// ```
///
/// Index with order
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let index = Index::create(Glyph::Table, (Glyph::Aspect, IndexOrder::Desc))
///     .name(Name::runtime("idx-glyph-aspect"))
///     .to_owned();
///
/// assert_eq!(
///     index.to_string(),
///     r#"CREATE INDEX "idx-glyph-aspect" ON "glyph" ("aspect" DESC)"#
/// );
/// ```
/// Index on multi-columns
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let index = Index::create(Glyph::Table, (Glyph::Image, IndexOrder::Asc))
///     .name(Name::runtime("idx-glyph-aspect"))
///     .col((Glyph::Aspect, IndexOrder::Desc))
///     .unique()
///     .to_owned();
///
/// assert_eq!(
///     index.to_string(),
///     r#"CREATE UNIQUE INDEX "idx-glyph-aspect" ON "glyph" ("image" ASC, "aspect" DESC)"#
/// );
/// ```
///
/// There is no `take()`: draining the table or the columns would leave exactly
/// the husk the constructor rules out, so the method a reader would expect to
/// move is absent rather than quietly copying. A second copy is `.to_owned()`.
///
/// ```compile_fail,E0599
/// use pgorm_query::{*, tests_cfg::*};
///
/// let mut index = Index::create(Glyph::Table, Glyph::Aspect).to_owned();
/// let moved: IndexCreateStatement = index.take();
/// ```
///
// [spec:pgorm:req:sql.ddl.index-create+11]
// [spec:pgorm:req:sql.ast+5]
#[derive(Debug, Clone)]
pub struct IndexCreateStatement {
    pub(crate) table: TableName,
    pub(crate) index: TableIndex,
    pub(crate) kind: IndexKind,
    pub(crate) nulls_not_distinct: bool,
    pub(crate) index_type: Option<IndexType>,
    pub(crate) if_not_exists: bool,
    pub(crate) include: Vec<Name>,
    pub(crate) r#where: ConditionHolder,
}

/// What an index constrains: nothing, or uniqueness.
///
/// There is no primary-key kind. PostgreSQL spells `PRIMARY KEY` only as a
/// table constraint, never as `CREATE INDEX`, so a primary key is the table's
/// [`TableCreateStatement::primary_key`](crate::TableCreateStatement::primary_key).
// [spec:pgorm:req:sql.ddl.index-create+11]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexKind {
    #[default]
    Plain,
    Unique,
}

/// The access method an index is built with — PostgreSQL's `USING <method>`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum IndexType {
    BTree,
    Gin,
    Hash,
    /// An access method this enum has no variant for, named rather than
    /// spelled: the name renders quoted-or-safe-bare, never as SQL.
    Named(Name),
}

impl IndexCreateStatement {
    /// Construct a new [`IndexCreateStatement`] over its table and first column
    pub fn new<T, C>(table: T, col: C) -> Self
    where
        T: IntoTableName,
        C: IntoIndexColumn,
    {
        let mut index = TableIndex::new();
        index.col(col.into_index_column());
        Self {
            table: table.into_table_name(),
            index,
            kind: IndexKind::default(),
            nulls_not_distinct: false,
            index_type: None,
            if_not_exists: false,
            include: Vec::new(),
            r#where: ConditionHolder::new(),
        }
    }

    /// Create index if index not exists
    pub fn if_not_exists(&mut self) -> &mut Self {
        self.if_not_exists = true;
        self
    }

    /// Set index name
    pub fn name<T>(&mut self, name: T) -> &mut Self
    where
        T: IntoName,
    {
        self.index.name(name);
        self
    }

    /// Add a further index column, after the one the constructor took
    pub fn col<C>(&mut self, col: C) -> &mut Self
    where
        C: IntoIndexColumn,
    {
        self.index.col(col.into_index_column());
        self
    }

    /// Set index kind to [`IndexKind::Unique`].
    pub fn unique(&mut self) -> &mut Self {
        self.kind = IndexKind::Unique;
        self
    }

    /// Set nulls to not be treated as distinct values.
    ///
    /// PostgreSQL defines this only for unique indexes, so it is rendered only
    /// when the kind is [`IndexKind::Unique`].
    pub fn nulls_not_distinct(&mut self) -> &mut Self {
        self.nulls_not_distinct = true;
        self
    }

    /// Build the index with the GIN access method — `USING GIN` — the one
    /// full-text search and the container operators (`@>`, `?`, `&&`) index
    /// under.
    pub fn gin(&mut self) -> &mut Self {
        self.index_type(IndexType::Gin)
    }

    /// Set the access method the index is built with. Omitted, PostgreSQL
    /// uses its default, `btree`, and the statement spells no `USING` clause.
    pub fn index_type(&mut self, index_type: IndexType) -> &mut Self {
        self.index_type = Some(index_type);
        self
    }

    /// Carry further columns in the index's leaves without indexing them —
    /// PostgreSQL's `INCLUDE`. Repeated calls append.
    ///
    /// An included column is not part of the key: it cannot be searched or
    /// ordered by, and exists so a query reading only these columns can be
    /// answered from the index alone. PostgreSQL allows it on plain and unique
    /// indexes alike, and on a unique index the included columns take no part
    /// in the uniqueness.
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Index::create(Glyph::Table, Glyph::Aspect)
    ///         .name(Name::runtime("idx-glyph-aspect"))
    ///         .include([Glyph::Image])
    ///         .to_string(),
    ///     r#"CREATE INDEX "idx-glyph-aspect" ON "glyph" ("aspect") INCLUDE ("image")"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.index-create+11]
    pub fn include<N, I>(&mut self, columns: I) -> &mut Self
    where
        N: IntoName,
        I: IntoIterator<Item = N>,
    {
        self.include
            .extend(columns.into_iter().map(IntoName::into_name));
        self
    }

    pub fn kind(&self) -> IndexKind {
        self.kind
    }

    pub fn is_unique_key(&self) -> bool {
        self.kind == IndexKind::Unique
    }

    pub fn is_nulls_not_distinct(&self) -> bool {
        self.nulls_not_distinct
    }

    pub fn get_index_spec(&self) -> &TableIndex {
        &self.index
    }

    pub fn get_table_name(&self) -> &TableName {
        &self.table
    }
}

/// Restrict the index to the rows a predicate accepts — PostgreSQL's partial
/// index. Repeated calls conjoin, as they do on every other statement that
/// carries a `WHERE`.
///
/// A partial unique index is the reason to reach for this: uniqueness holds
/// among the matching rows and nowhere else, which is how "one active row per
/// owner" is spelled without a constraint over the whole table. The predicate
/// may only read the indexed table's own columns.
///
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// assert_eq!(
///     Index::create(Glyph::Table, Glyph::Aspect)
///         .name(Name::runtime("idx-glyph-aspect-live"))
///         .unique()
///         .and_where(Expr::col(Glyph::Image).is_not_null())
///         .to_string(),
///     [
///         r#"CREATE UNIQUE INDEX "idx-glyph-aspect-live" ON "glyph" ("aspect")"#,
///         r#"WHERE "image" IS NOT NULL"#,
///     ]
///     .join(" ")
/// );
/// ```
// [spec:pgorm:req:sql.ddl.index-create+11]
#[inherent]
impl ConditionalStatement for IndexCreateStatement {
    pub fn cond_where<C>(&mut self, condition: C) -> &mut Self
    where
        C: IntoCondition,
    {
        self.r#where.add_condition(condition.into_condition());
        self
    }

    pub fn and_where_option(&mut self, other: Option<SimpleExpr>) -> &mut Self;
    pub fn and_where(&mut self, other: SimpleExpr) -> &mut Self;
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for IndexCreateStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_index_create_statement(self, &mut sql);
        f.write_str(&sql)
    }
}
