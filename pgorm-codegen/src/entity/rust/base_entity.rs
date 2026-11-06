use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::ident::safe_ident;
use crate::{Entity, Error, util::escape_rust_keyword};

impl Entity {
    /// Every Rust identifier the writer derives from this entity's DB names,
    /// checked by the Rust writer's gate while the caller can still be handed
    /// the failure.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    // [spec:pgorm:sem:codegen.entity.keywords+2]
    pub(crate) fn validate_idents(&self) -> Result<(), Error> {
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
            column
                .validate_idents()
                .map_err(|err| err.in_table(table))?;
        }
        for primary_key in self.primary_keys.iter() {
            primary_key.validate().map_err(|err| err.in_table(table))?;
        }
        for relation in self.relations.iter() {
            relation
                .validate_idents()
                .map_err(|err| err.in_table(table))?;
        }
        for conjunct_relation in self.conjunct_relations.iter() {
            conjunct_relation
                .validate()
                .map_err(|err| err.in_table(table))?;
        }
        Ok(())
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

    pub fn get_conjunct_relations_to_camel_case(&self) -> Vec<Ident> {
        self.conjunct_relations
            .iter()
            .map(|con_rel| con_rel.get_to_upper_camel_case())
            .collect()
    }

    // [spec:pgorm:sem:codegen.entity.types+13]
    pub fn get_eq_needed(&self) -> TokenStream {
        if self.has_float() {
            quote! {}
        } else {
            quote! {, Eq}
        }
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
    use quote::{format_ident, quote};

    use crate::entity::base_entity::tests::setup;

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
    fn conjunct_relations_to_camel_case() {
        let entity = setup();

        for (i, elem) in entity
            .get_conjunct_relations_to_camel_case()
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
