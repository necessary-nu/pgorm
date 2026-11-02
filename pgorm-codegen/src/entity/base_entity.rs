use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{ColumnType, TableName};
use proc_macro2::{Ident, TokenStream};
use quote::format_ident;
use quote::quote;
use std::fmt;

use crate::{
    Column, ConjunctRelation, Error, PrimaryKey, Relation,
    util::{escape_rust_keyword, safe_ident},
};

/// Which table a name names: the bare table name, and the schema its DDL
/// qualified it with, when it carried one.
///
/// Ordered by bare name first, so the entity map — and every output that walks
/// it — stays alphabetical by table name however the schemas sort.
// [spec:pgorm:sem:codegen.entity.transform+16]
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableIdent {
    pub table: String,
    pub schema: Option<String>,
}

impl TableIdent {
    /// The identity a DDL statement's table name spells.
    // [spec:pgorm:sem:codegen.entity.transform+16]
    pub fn of(name: &TableName) -> Self {
        Self {
            table: name.table().to_string(),
            schema: name.schema().map(|schema| schema.to_string()),
        }
    }
}

/// As the schema wrote it: `schema.table`, or `table` when unqualified.
// [spec:pgorm:sem:codegen.entity.transform+16]
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
    // [spec:pgorm:sem:codegen.entity.transform+16]
    pub(crate) without_overlaps: bool,
}

impl Entity {
    /// Which table this entity is generated from.
    // [spec:pgorm:sem:codegen.entity.transform+16]
    pub fn ident(&self) -> TableIdent {
        TableIdent {
            table: self.table_name.clone(),
            schema: self.schema_name.clone(),
        }
    }

    /// The schema the generated entity names: the source table's own qualifier
    /// when it had one, else the configured default. A fact the schema states
    /// is never overridden by an option.
    // [spec:pgorm:sem:codegen.entity.transform+16]
    pub fn effective_schema<'a>(&'a self, default: &'a Option<String>) -> Option<&'a str> {
        self.schema_name.as_deref().or(default.as_deref())
    }

    /// Everything the writer derives from this entity's DB names, checked while
    /// the caller can still be handed the failure.
    // [spec:pgorm:sem:codegen.entity.transform+16]
    // [spec:pgorm:sem:codegen.entity.keywords+2]
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let qualified = self.ident().to_string();
        let table = qualified.as_str();
        let context = format!("table `{table}`");
        safe_ident(
            &context,
            &escape_rust_keyword(self.get_table_name_snake_case()),
        )?;
        safe_ident(
            &context,
            &escape_rust_keyword(self.get_table_name_camel_case()),
        )?;
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
            primary_key.validate().map_err(|err| err.in_table(table))?;
        }
        for relation in self.relations.iter() {
            relation.validate().map_err(|err| err.in_table(table))?;
        }
        for conjunct_relation in self.conjunct_relations.iter() {
            conjunct_relation
                .validate()
                .map_err(|err| err.in_table(table))?;
        }
        Ok(())
    }

    pub fn get_table_name_snake_case(&self) -> String {
        self.table_name.to_snake_case()
    }

    pub fn get_table_name_camel_case(&self) -> String {
        self.table_name.to_upper_camel_case()
    }

    pub fn get_table_name_snake_case_ident(&self) -> Ident {
        format_ident!("{}", escape_rust_keyword(self.get_table_name_snake_case()))
    }

    pub fn get_table_name_camel_case_ident(&self) -> Ident {
        format_ident!("{}", escape_rust_keyword(self.get_table_name_camel_case()))
    }

    pub fn get_column_names_snake_case(&self) -> Vec<Ident> {
        self.columns
            .iter()
            .map(|col| col.get_name_snake_case())
            .collect()
    }

    pub fn get_column_rs_types(&self) -> Vec<TokenStream> {
        self.columns
            .clone()
            .into_iter()
            .map(|col| col.get_rs_type())
            .collect()
    }

    pub fn get_primary_key_names_snake_case(&self) -> Vec<Ident> {
        self.primary_keys
            .iter()
            .map(|pk| pk.get_name_snake_case())
            .collect()
    }

    pub fn get_relation_module_name(&self) -> Vec<Option<Ident>> {
        self.relations
            .iter()
            .map(|rel| rel.get_module_name())
            .collect()
    }

    pub fn get_relation_enum_name(&self) -> Vec<Ident> {
        self.relations
            .iter()
            .map(|rel| rel.get_enum_name())
            .collect()
    }

    pub fn get_relation_attrs(&self) -> Vec<TokenStream> {
        self.relations.iter().map(|rel| rel.get_attrs()).collect()
    }

    pub fn get_conjunct_relations_via_snake_case(&self) -> Vec<Ident> {
        self.conjunct_relations
            .iter()
            .map(|con_rel| con_rel.get_via_snake_case())
            .collect()
    }

    pub fn get_conjunct_relations_to_snake_case(&self) -> Vec<Ident> {
        self.conjunct_relations
            .iter()
            .map(|con_rel| con_rel.get_to_snake_case())
            .collect()
    }

    pub fn get_conjunct_relations_to_upper_camel_case(&self) -> Vec<Ident> {
        self.conjunct_relations
            .iter()
            .map(|con_rel| con_rel.get_to_upper_camel_case())
            .collect()
    }

    // [spec:pgorm:sem:codegen.entity.types+12]
    pub fn get_eq_needed(&self) -> TokenStream {
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
            .find(|column| is_floats(&column.col_type))
            // check if float or double exist.
            // if exist, return nothing
            .map_or(quote! {, Eq}, |_| quote! {})
    }

    pub fn get_column_serde_attributes(
        &self,
        serde_skip_deserializing_primary_key: bool,
        serde_skip_hidden_column: bool,
    ) -> Vec<TokenStream> {
        self.columns
            .iter()
            .map(|col| {
                let is_primary_key = self.primary_keys.iter().any(|pk| pk.name == col.name);
                col.get_serde_attribute(
                    is_primary_key,
                    serde_skip_deserializing_primary_key,
                    serde_skip_hidden_column,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use pgorm_query::{ColumnType, ForeignKeyAction, StringLen};
    use quote::{format_ident, quote};

    use crate::{Column, Entity, PrimaryKey, Relation, RelationType};

    fn setup() -> Entity {
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

    #[test]
    fn test_get_table_name_snake_case_ident() {
        let entity = setup();

        assert_eq!(
            entity.get_table_name_snake_case_ident(),
            format_ident!("{}", "special_cake")
        );
    }

    #[test]
    fn test_get_table_name_camel_case_ident() {
        let entity = setup();

        assert_eq!(
            entity.get_table_name_camel_case_ident(),
            format_ident!("{}", "SpecialCake")
        );
    }

    #[test]
    fn test_get_column_names_snake_case() {
        let entity = setup();

        for (i, elem) in entity.get_column_names_snake_case().into_iter().enumerate() {
            assert_eq!(elem, entity.columns[i].get_name_snake_case());
        }
    }

    #[test]
    fn test_get_column_rs_types() {
        let entity = setup();

        for (i, elem) in entity.get_column_rs_types().into_iter().enumerate() {
            assert_eq!(
                elem.to_string(),
                entity.columns[i].get_rs_type().to_string()
            );
        }
    }

    #[test]
    fn test_get_primary_key_names_snake_case() {
        let entity = setup();

        for (i, elem) in entity
            .get_primary_key_names_snake_case()
            .into_iter()
            .enumerate()
        {
            assert_eq!(elem, entity.primary_keys[i].get_name_snake_case());
        }
    }

    #[test]
    fn test_get_relation_module_name() {
        let entity = setup();

        for (i, elem) in entity.get_relation_module_name().into_iter().enumerate() {
            assert_eq!(elem, entity.relations[i].get_module_name());
        }
    }

    #[test]
    fn test_get_relation_enum_name() {
        let entity = setup();

        for (i, elem) in entity.get_relation_enum_name().into_iter().enumerate() {
            assert_eq!(elem, entity.relations[i].get_enum_name());
        }
    }

    #[test]
    fn test_get_relation_attrs() {
        let entity = setup();

        for (i, elem) in entity.get_relation_attrs().into_iter().enumerate() {
            assert_eq!(
                elem.to_string(),
                entity.relations[i].get_attrs().to_string()
            );
        }
    }

    #[test]
    fn test_get_conjunct_relations_via_snake_case() {
        let entity = setup();

        for (i, elem) in entity
            .get_conjunct_relations_via_snake_case()
            .into_iter()
            .enumerate()
        {
            assert_eq!(elem, entity.conjunct_relations[i].get_via_snake_case());
        }
    }

    #[test]
    fn test_get_conjunct_relations_to_snake_case() {
        let entity = setup();

        for (i, elem) in entity
            .get_conjunct_relations_to_snake_case()
            .into_iter()
            .enumerate()
        {
            assert_eq!(elem, entity.conjunct_relations[i].get_to_snake_case());
        }
    }

    #[test]
    fn test_get_conjunct_relations_to_upper_camel_case() {
        let entity = setup();

        for (i, elem) in entity
            .get_conjunct_relations_to_upper_camel_case()
            .into_iter()
            .enumerate()
        {
            assert_eq!(elem, entity.conjunct_relations[i].get_to_upper_camel_case());
        }
    }

    #[test]
    fn test_get_eq_needed() {
        let entity = setup();
        let expected = quote! {, Eq};

        assert_eq!(entity.get_eq_needed().to_string(), expected.to_string());
    }
}
