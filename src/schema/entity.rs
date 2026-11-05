use crate::{
    ActiveEnum, ColumnTrait, ColumnType, EntityTrait, Error, Iterable, PrimaryKeyArity,
    PrimaryKeyToColumn, PrimaryKeyTrait, RelationDef, RelationTrait, Schema, entity::ColumnDefault,
    executor::result_name::derived_name,
};
use pgorm_query::{
    ColumnDef, Comment, CommentStatement, ForeignKeyCreateStatement, IdentityGeneration, Index,
    IndexCreateStatement, Name, SqlName, TableCreateStatement, TableKey, Unique,
    extension::{IntoTypeRef, Type, TypeCreateStatement},
};
use std::collections::HashSet;

impl Schema {
    /// Creates Postgres enums from an ActiveEnum. See [TypeCreateStatement] for more details.
    ///
    /// An `ActiveEnum` may be backed by a plain column type — a `String` column,
    /// say — rather than a database enum, in which case there is no type to
    /// create and this returns [`Error::Type`].
    pub fn create_enum_from_active_enum<A>(&self) -> Result<TypeCreateStatement, Error>
    where
        A: ActiveEnum,
    {
        create_enum_from_active_enum::<A>()
    }

    /// Creates Postgres enums from an Entity. See [TypeCreateStatement] for more details
    pub fn create_enum_from_entity<E>(&self, entity: E) -> Vec<TypeCreateStatement>
    where
        E: EntityTrait,
    {
        create_enum_from_entity(entity)
    }

    /// Creates a table from an Entity. See [TableCreateStatement] for more details.
    ///
    /// Each belongs-to relation becomes a foreign key. One PostgreSQL cannot
    /// implement is refused with [`Error::Type`] naming the relation rather
    /// than written: the server keys no column typed as a domain over an
    /// enum, on either side of the key, not even to the enum itself (`42804`).
    /// The check reads each side's type from the
    /// [`ColumnPairs`](crate::ColumnPairs) of the relation, which records it
    /// only when the pair was named by entity columns, as the `Relation` derive and
    /// [`RelationBuilder`](crate::RelationBuilder) name them: a pair built from
    /// bare names is written as declared, unchecked.
    pub fn create_table_from_entity<E>(&self, entity: E) -> Result<TableCreateStatement, Error>
    where
        E: EntityTrait,
    {
        create_table_from_entity(entity)
    }

    /// Creates the indexes from an Entity, returning an empty Vec if there are none
    /// to create. See [IndexCreateStatement] for more details
    pub fn create_index_from_entity<E>(&self, entity: E) -> Vec<IndexCreateStatement>
    where
        E: EntityTrait,
    {
        create_index_from_entity(entity)
    }

    /// Creates the comments from an Entity, returning an empty Vec if neither the
    /// entity nor any of its columns declares one. A comment is a statement of its
    /// own in Postgres, so these are executed alongside — not as part of — the
    /// statement from [`Schema::create_table_from_entity`].
    /// See [CommentStatement] for more details.
    ///
    /// ```
    /// use pgorm::{Schema, entity::prelude::*};
    ///
    /// #[derive(Clone, Debug, PartialEq, Entity)]
    /// #[pgorm(table_name = "posts", comment = "one row per post")]
    /// pub struct Model {
    ///     #[pgorm(primary_key)]
    ///     pub id: i32,
    ///     #[pgorm(comment = "the author's title")]
    ///     pub title: String,
    /// }
    ///
    /// #[derive(Copy, Clone, Debug, Relation)]
    /// pub enum Relation {}
    ///
    /// let schema = Schema::new();
    /// let comments: Vec<String> = schema
    ///     .create_comments_from_entity(Entity)
    ///     .iter()
    ///     .map(|stmt| stmt.to_string())
    ///     .collect();
    ///
    /// assert_eq!(
    ///     comments,
    ///     [
    ///         r#"COMMENT ON TABLE "posts" IS 'one row per post'"#,
    ///         r#"COMMENT ON COLUMN "posts"."title" IS 'the author''s title'"#,
    ///     ]
    /// );
    /// ```
    pub fn create_comments_from_entity<E>(&self, entity: E) -> Vec<CommentStatement>
    where
        E: EntityTrait,
    {
        create_comments_from_entity(entity)
    }

    /// Creates a column definition for example to update a table.
    ///
    /// ```
    /// use pgorm::{Schema, entity::prelude::*};
    /// use pgorm_query::Table;
    ///
    /// #[derive(Clone, Debug, PartialEq, Entity)]
    /// #[pgorm(table_name = "posts")]
    /// pub struct Model {
    ///     #[pgorm(primary_key)]
    ///     pub id: i32,
    ///     pub title: String,
    /// }
    ///
    /// #[derive(Copy, Clone, Debug, Relation)]
    /// pub enum Relation {}
    ///
    /// let schema = Schema::new();
    ///
    /// let alter_table = Table::alter(Entity)
    ///     .add_column(&mut schema.get_column_def::<Entity>(Column::Title));
    /// assert_eq!(
    ///     alter_table.to_string(),
    ///     r#"ALTER TABLE "posts" ADD COLUMN "title" varchar NOT NULL"#
    /// );
    /// ```
    pub fn get_column_def<E>(&self, column: E::Column) -> ColumnDef
    where
        E: EntityTrait,
    {
        column_def_from_entity_column::<E>(column)
    }
}

// [spec:pgorm:sem:schema.from-entity.enum+3]    from a single ActiveEnum
pub(crate) fn create_enum_from_active_enum<A>() -> Result<TypeCreateStatement, Error>
where
    A: ActiveEnum,
{
    let col_def = A::db_type();
    create_enum_from_column_type(col_def.get_column_type()).ok_or_else(|| {
        Error::Type(format!(
            "`{}` is not backed by a database enum, so there is no type to create",
            A::name().to_string()
        ))
    })
}

/// Looks through one level of [`ColumnType::Array`], so a column holding an
/// array of a database enum resolves to the enum the array is over.
fn enum_element_type(col_type: &ColumnType) -> &ColumnType {
    match col_type {
        ColumnType::Array(inner) => inner,
        _ => col_type,
    }
}

pub(crate) fn create_enum_from_column_type(col_type: &ColumnType) -> Option<TypeCreateStatement> {
    let ColumnType::Enum {
        name,
        schema,
        variants,
    } = enum_element_type(col_type)
    else {
        return None;
    };
    let type_ref = match schema {
        Some(schema) => (schema.clone(), name.clone()).into_type_ref(),
        None => name.clone().into_type_ref(),
    };
    // Labels are data, not names: `TypeCreateStatement::values` renders them
    // as string literals, so the variant idens are flattened to their text.
    // [spec:pgorm:req:sql.render.ddl.enum-type+6]
    Some(
        Type::create(type_ref)
            .values(variants.iter().map(|v| v.to_string()))
            .to_owned(),
    )
}

// [spec:pgorm:sem:schema.from-entity.enum+3]
pub(crate) fn create_enum_from_entity<E>(_: E) -> Vec<TypeCreateStatement>
where
    E: EntityTrait,
{
    let mut seen = HashSet::new();
    let mut vec = Vec::new();
    for col in E::Column::iter() {
        let col_def = col.def();
        let col_type = enum_element_type(col_def.get_column_type());
        let ColumnType::Enum { name, schema, .. } = col_type else {
            continue;
        };
        let seen_key = match schema {
            Some(schema) => format!("{}.{}", schema.to_string(), name.to_string()),
            None => name.to_string(),
        };
        if seen.insert(seen_key) {
            vec.extend(create_enum_from_column_type(col_type));
        }
    }
    vec
}

// [spec:pgorm:sem:schema.from-entity.index+2]
pub(crate) fn create_index_from_entity<E>(entity: E) -> Vec<IndexCreateStatement>
where
    E: EntityTrait,
{
    let mut vec = Vec::new();
    for column in E::Column::iter() {
        let column_def = column.def();
        if !column_def.indexed {
            continue;
        }
        let name = derived_name("idx", &entity.to_string(), [column.to_string().as_str()]);
        let stmt = Index::create(entity.table_ref(), column)
            .name(Name::runtime(name))
            .to_owned();
        vec.push(stmt)
    }
    vec
}

// [spec:pgorm:sem:schema.from-entity+12]    the comment statements, one stream per entity
pub(crate) fn create_comments_from_entity<E>(entity: E) -> Vec<CommentStatement>
where
    E: EntityTrait,
{
    let table = entity.table_ref();

    let mut vec = Vec::new();
    if let Some(comment) = entity.comment() {
        vec.push(Comment::on_table(table.clone(), comment));
    }
    for column in E::Column::iter() {
        if let Some(comment) = column.def().comment {
            vec.push(Comment::on_column(table.clone(), column, comment));
        }
    }
    vec
}

// [spec:pgorm:sem:schema.from-entity+12]
pub(crate) fn create_table_from_entity<E>(entity: E) -> Result<TableCreateStatement, Error>
where
    E: EntityTrait,
{
    table_from_entity(entity, |_| true)
}

/// The table [`create_table_from_entity`] writes, with a foreign key for each
/// belongs-to relation `writes_key` keeps: a disjoint kind's table leaves out
/// the one its key to its base row implies.
// [spec:pgorm:sem:schema.from-entity+12]
// [spec:pgorm:sem:disjoint.schema+2]
pub(crate) fn table_from_entity<E, K>(
    entity: E,
    writes_key: K,
) -> Result<TableCreateStatement, Error>
where
    E: EntityTrait,
    K: Fn(&RelationDef) -> bool,
{
    let mut stmt = TableCreateStatement::new(entity.table_ref());

    if let Some(comment) = entity.comment() {
        stmt.comment(comment);
    }

    for column in E::Column::iter() {
        let mut column_def = column_def_from_entity_column::<E>(column);
        stmt.col(&mut column_def);
    }

    let mut primary_keys: Vec<Name> = E::PrimaryKey::iter().map(Name::new).collect();
    // A temporal key's period is its last column, which the key writes
    // `WITHOUT OVERLAPS` after the rest.
    let period = match E::PrimaryKey::without_overlaps() {
        true => primary_keys.pop(),
        false => None,
    };
    write_primary_key(&mut stmt, &entity.to_string(), primary_keys, period);
    for column in E::Column::iter() {
        if column.def().unique {
            stmt.unique(column);
        }
    }
    for key in entity.unique_keys() {
        stmt.unique(unique_key(
            &entity.to_string(),
            key.name,
            key.columns,
            key.nulls_not_distinct,
        )?);
    }

    for relation in E::Relation::iter() {
        let def = relation.def();
        if def.is_owner || !writes_key(&def) {
            continue;
        }
        refuse_unkeyable(&entity, &relation, &def)?;
        stmt.foreign_key(ForeignKeyCreateStatement::from(def));
    }

    Ok(stmt.take())
}

/// The unique key of the table `label` over `columns`, in key order, named
/// `name` or left for PostgreSQL to name: [`Error::Type`] for one naming no
/// column, which an `EntityName` written by hand or a table described at run
/// time can describe and PostgreSQL cannot write (`UNIQUE ()`).
// [spec:pgorm:sem:schema.from-entity+12]
// [spec:pgorm:req:bindings.model-unique-keys+1]
pub(crate) fn unique_key(
    label: &str,
    name: Option<&str>,
    columns: &[&str],
    nulls_not_distinct: bool,
) -> Result<TableKey<Unique>, Error> {
    let Some((first, rest)) = columns.split_first() else {
        return Err(Error::Type(format!(
            "`{label}` declares a unique key with no columns"
        )));
    };
    let mut table_key =
        TableKey::new(Name::runtime(*first)).cols(rest.iter().copied().map(Name::runtime));
    if let Some(name) = name {
        table_key = table_key.name(Name::runtime(name));
    }
    if nulls_not_distinct {
        table_key = table_key.nulls_not_distinct();
    }
    Ok(table_key)
}

/// Refuse a foreign key PostgreSQL cannot implement: one matching a pair
/// with a column typed as a domain over an enum on either side, which the
/// server answers `42804` (incompatible types) whatever the other side is —
/// the same domain, another domain over the enum, or the enum itself.
///
/// Each side's type is the one its [`ColumnPairs`](crate::ColumnPairs)
/// recorded when the pair was named by entity columns; a pair named by bare
/// names recorded none and passes unchecked.
// [spec:pgorm:sem:schema.from-entity+12]
fn refuse_unkeyable<E: EntityTrait>(
    entity: &E,
    relation: &E::Relation,
    def: &RelationDef,
) -> Result<(), Error> {
    let unkeyable = def
        .columns
        .compared_pairs()
        .find(|(_, compared)| compared.from.is_some() || compared.to.is_some());
    let Some(((from, to), _)) = unkeyable else {
        return Ok(());
    };
    Err(Error::Type(format!(
        "relation `{relation:?}` of `{entity}` cannot be a foreign key: `{from_tbl}`.`{from}` \
         references `{to_tbl}`.`{to}`, and PostgreSQL implements no foreign key with a column \
         typed as a domain over an enum on either side (42804)",
        entity = entity.to_string(),
        from_tbl = def.from_tbl.qualifier().to_string(),
        from = from.to_string(),
        to_tbl = def.to_tbl.qualifier().to_string(),
        to = to.to_string(),
    )))
}

/// Write the primary key of `keys`, then `period` written `WITHOUT
/// OVERLAPS`, onto `stmt`, the table `table`'s: a composite key named
/// `pk-{table}`, a one-column key keeping the name PostgreSQL derives,
/// `{table}_pkey`.
// [spec:pgorm:sem:schema.from-entity+12]
pub(crate) fn write_primary_key(
    stmt: &mut TableCreateStatement,
    table: &str,
    keys: Vec<Name>,
    period: Option<Name>,
) {
    let composite = keys.len() + usize::from(period.is_some()) > 1;
    let mut keys = keys.into_iter();
    let Some(first) = keys.next() else {
        return;
    };
    let mut key = TableKey::new(first).cols(keys);
    if let Some(period) = period {
        key = key.without_overlaps(period);
    }
    if composite {
        stmt.primary_key(key.name(Name::runtime(derived_name("pk", table, []))));
    } else {
        stmt.primary_key(key);
    }
}

// [spec:pgorm:sem:schema.from-entity+12]    column projection, and the serial family for a one-column key
fn column_def_from_entity_column<E>(column: E::Column) -> ColumnDef
where
    E: EntityTrait,
{
    // Only a one-column key is drawn from the serial family, and only when the
    // column names no fill of its own: PostgreSQL refuses `serial` beside a
    // `DEFAULT`, an identity or a generation expression (42601). A composite
    // key's generated part is its column's identity.
    let serial = <<E::PrimaryKey as PrimaryKeyTrait>::ValueType as PrimaryKeyArity>::ARITY == 1
        && E::PrimaryKey::auto_increment()
        && E::PrimaryKey::iter().any(|key| column.to_string() == key.into_column().to_string());
    column_ddl(Name::new(column), column.def(), serial)
}

/// The column `name` of the definition `orm_column_def` as a `CREATE TABLE`
/// writes it, drawn from the serial family when `serial` holds and the
/// column names no fill of its own.
// [spec:pgorm:sem:schema.from-entity+12]
pub(crate) fn column_ddl(name: Name, orm_column_def: crate::ColumnDef, serial: bool) -> ColumnDef {
    let mut column_def = ColumnDef::new_with_type(name, orm_column_def.col_type);
    if !orm_column_def.null {
        column_def.not_null();
    }
    let fills_itself = orm_column_def.default.is_some();
    match orm_column_def.default {
        Some(ColumnDefault::Expr(default)) => {
            column_def.default(default);
        }
        Some(ColumnDefault::Identity(IdentityGeneration::Always)) => {
            column_def.identity();
        }
        Some(ColumnDefault::Identity(IdentityGeneration::ByDefault)) => {
            column_def.identity_by_default();
        }
        Some(ColumnDefault::Generated(expr, kind)) => {
            column_def.generated(expr, kind);
        }
        None => {}
    }
    if let Some(comment) = orm_column_def.comment {
        column_def.comment(comment);
    }
    if serial && !fills_itself {
        column_def.auto_increment();
    }
    column_def
}

#[cfg(test)]
mod tests {
    use crate::{EntityName, Schema, pgorm_query::*, tests_cfg::*};
    use pretty_assertions::assert_eq;

    #[test]
    fn test_create_table_from_entity_table_ref() {
        let schema = Schema::new();
        assert_eq!(
            schema
                .create_table_from_entity(CakeFillingPrice)
                .expect("the table is generated")
                .to_string(),
            get_cake_filling_price_stmt().to_string()
        );
    }

    fn get_cake_filling_price_stmt() -> TableCreateStatement {
        Table::create(CakeFillingPrice.table_ref())
            .col(
                ColumnDef::new(cake_filling_price::Column::CakeId)
                    .integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(cake_filling_price::Column::FillingId)
                    .integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(cake_filling_price::Column::Price)
                    .decimal()
                    .not_null(),
            )
            .primary_key(
                TableKey::new(cake_filling_price::Column::CakeId)
                    .name(Name::runtime("pk-cake_filling_price"))
                    .col(cake_filling_price::Column::FillingId),
            )
            .foreign_key(
                ForeignKeyCreateStatement::new(
                    CakeFillingPrice,
                    cake_filling_price::Column::CakeId,
                    CakeFilling,
                    cake_filling::Column::CakeId,
                )
                .col(
                    cake_filling_price::Column::FillingId,
                    cake_filling::Column::FillingId,
                )
                .name(Name::runtime("fk-cake_filling_price-cake_id-filling_id"))
                .to_owned(),
            )
            .to_owned()
    }

    #[test]
    fn test_create_index_from_entity_table_ref() {
        let schema = Schema::new();

        assert_eq!(
            schema
                .create_table_from_entity(indexes::Entity)
                .expect("the table is generated")
                .to_string(),
            get_indexes_stmt().to_string()
        );

        let stmts = schema.create_index_from_entity(indexes::Entity);
        assert_eq!(stmts.len(), 2);

        let idx: IndexCreateStatement =
            Index::create(indexes::Entity.table_ref(), indexes::Column::Index1Attr)
                .name(Name::runtime("idx-indexes-index1_attr"))
                .to_owned();
        assert_eq!(stmts[0].to_string(), idx.to_string());

        let idx: IndexCreateStatement =
            Index::create(indexes::Entity.table_ref(), indexes::Column::Index2Attr)
                .name(Name::runtime("idx-indexes-index2_attr"))
                .to_owned();
        assert_eq!(stmts[1].to_string(), idx.to_string());
    }

    fn get_indexes_stmt() -> TableCreateStatement {
        Table::create(indexes::Entity.table_ref())
            .col(
                ColumnDef::new(indexes::Column::IndexesId)
                    .integer()
                    .not_null()
                    .auto_increment(),
            )
            .primary_key(indexes::Column::IndexesId)
            .col(
                ColumnDef::new(indexes::Column::UniqueAttr)
                    .integer()
                    .not_null(),
            )
            .unique(indexes::Column::UniqueAttr)
            .col(
                ColumnDef::new(indexes::Column::Index1Attr)
                    .integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(indexes::Column::Index2Attr)
                    .integer()
                    .not_null(),
            )
            .unique(indexes::Column::Index2Attr)
            .to_owned()
    }
}
