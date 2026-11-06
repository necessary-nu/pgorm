use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{ColumnType, TableName};
use std::fmt;

use crate::{
    Column, ConjunctRelation, DisjointEnum, Error, OneOfEnum, PrimaryKey, Relation, UniqueKey,
};

/// Which table a name names: the bare table name, and the schema its DDL
/// qualified it with, when it carried one.
///
/// Ordered by bare name first, so the entity map — and every output that walks
/// it — stays alphabetical by table name however the schemas sort.
// [spec:pgorm:sem:codegen.entity.transform+18]
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableIdent {
    pub table: String,
    pub schema: Option<String>,
}

impl TableIdent {
    /// The identity a DDL statement's table name spells.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    pub fn of(name: &TableName) -> Self {
        Self {
            table: name.table().to_string(),
            schema: name.schema().map(|schema| schema.to_string()),
        }
    }
}

/// As the schema wrote it: `schema.table`, or `table` when unqualified.
// [spec:pgorm:sem:codegen.entity.transform+18]
impl fmt::Display for TableIdent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.schema {
            Some(schema) => write!(f, "{schema}.{}", self.table),
            None => f.write_str(&self.table),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entity {
    pub(crate) table_name: String,
    /// The schema the source table was qualified with, kept as written.
    pub(crate) schema_name: Option<String>,
    pub(crate) columns: Vec<Column>,
    pub(crate) relations: Vec<Relation>,
    pub(crate) conjunct_relations: Vec<ConjunctRelation>,
    pub(crate) primary_keys: Vec<PrimaryKey>,
    /// Whether the last primary-key column is the key's period, PostgreSQL
    /// 18's `WITHOUT OVERLAPS`.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    pub(crate) without_overlaps: bool,
    /// The disjoint enum the entity's module declares, when its table is
    /// the base of one.
    // [spec:pgorm:sem:codegen.entity.disjoint+3]
    pub(crate) disjoint: Option<DisjointEnum>,
    /// The `#[derive(OneOf)]` enums the entity's module declares, one per
    /// `CHECK` holding nullable foreign-key columns to one target.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub(crate) one_of: Vec<OneOfEnum>,
    /// The table's unique keys over two columns or more, in the table's
    /// order, which the entity declares with `unique(..)`.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    pub(crate) unique_keys: Vec<UniqueKey>,
}

impl Entity {
    /// Which table this entity is generated from.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    pub fn ident(&self) -> TableIdent {
        TableIdent {
            table: self.table_name.clone(),
            schema: self.schema_name.clone(),
        }
    }

    /// The schema the generated entity names: the source table's own qualifier
    /// when it had one, else the configured default. A fact the schema states
    /// is never overridden by an option.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    pub fn effective_schema<'a>(&'a self, default: &'a Option<String>) -> Option<&'a str> {
        self.schema_name.as_deref().or(default.as_deref())
    }

    /// What a writer could not write of this entity, checked while the caller
    /// can still be handed the failure: a column of a type outside the
    /// mapping, a primary-key column that is not a column of the table, a
    /// relation that is not written. The Rust identifiers its DB names derive
    /// are the Rust writer's gate's to check.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let qualified = self.ident().to_string();
        let table = qualified.as_str();
        let context = format!("table `{table}`");
        for column in self.columns.iter() {
            column.validate().map_err(|err| err.in_table(table))?;
        }
        for primary_key in self.primary_keys.iter() {
            if !self.columns.iter().any(|col| col.name == primary_key.name) {
                return Err(Error::TransformError(format!(
                    "{context}: primary key column `{}` is not a column of the table",
                    primary_key.name
                )));
            }
        }
        for relation in self.relations.iter() {
            relation.validate().map_err(|err| err.in_table(table))?;
        }
        Ok(())
    }

    pub fn get_table_name_snake_case(&self) -> String {
        self.table_name.to_snake_case()
    }

    pub fn get_table_name_camel_case(&self) -> String {
        self.table_name.to_upper_camel_case()
    }

    /// Whether a column holds a float, through arrays, created ranges and
    /// domains: a float is not `Eq`, so neither is a row holding one.
    // [spec:pgorm:sem:codegen.entity.types+13]
    pub(crate) fn has_float(&self) -> bool {
        fn is_floats(col_type: &ColumnType) -> bool {
            match col_type {
                ColumnType::Float | ColumnType::Double => true,
                ColumnType::Array(col_type) => is_floats(col_type),
                ColumnType::CreatedRange { subtype, .. }
                | ColumnType::CreatedMultirange { subtype, .. } => is_floats(subtype),
                ColumnType::Domain { base, .. } => is_floats(base),
                _ => false,
            }
        }
        self.columns
            .iter()
            .any(|column| is_floats(&column.col_type))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use pgorm_query::{ColumnType, ForeignKeyAction, StringLen};

    use crate::{Column, Entity, PrimaryKey, Relation, RelationType};

    pub(crate) fn setup() -> Entity {
        Entity {
            table_name: "special_cake".to_owned(),
            schema_name: None,
            columns: vec![
                Column {
                    name: "id".to_owned(),
                    col_type: ColumnType::Integer,
                    auto_increment: false,
                    identity: None,
                    not_null: false,
                    unique: false,
                    default: None,
                    generated: None,
                    enum_home: None,
                },
                Column {
                    name: "name".to_owned(),
                    col_type: ColumnType::String(StringLen::None),
                    auto_increment: false,
                    identity: None,
                    not_null: false,
                    unique: false,
                    default: None,
                    generated: None,
                    enum_home: None,
                },
            ],
            relations: vec![
                Relation {
                    ref_table: "fruit".to_owned(),
                    ref_schema: None,
                    columns: vec!["id".to_owned()],
                    ref_columns: vec!["cake_id".to_owned()],
                    rel_type: RelationType::HasOne,
                    on_delete: Some(ForeignKeyAction::Cascade),
                    on_update: Some(ForeignKeyAction::Cascade),
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                },
                Relation {
                    ref_table: "filling".to_owned(),
                    ref_schema: None,
                    columns: vec!["id".to_owned()],
                    ref_columns: vec!["cake_id".to_owned()],
                    rel_type: RelationType::HasOne,
                    on_delete: Some(ForeignKeyAction::Cascade),
                    on_update: Some(ForeignKeyAction::Cascade),
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                },
            ],
            conjunct_relations: vec![],
            without_overlaps: false,
            disjoint: None,
            one_of: Vec::new(),
            unique_keys: Vec::new(),
            primary_keys: vec![PrimaryKey {
                name: "id".to_owned(),
            }],
        }
    }

    #[test]
    fn test_get_table_name_snake_case() {
        let entity = setup();

        assert_eq!(
            entity.get_table_name_snake_case(),
            "special_cake".to_owned()
        );
    }

    #[test]
    fn test_get_table_name_camel_case() {
        let entity = setup();

        assert_eq!(entity.get_table_name_camel_case(), "SpecialCake".to_owned());
    }
}
