use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{Deferrability, Enforcement, ForeignKeyAction};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::ident::safe_ident;
use crate::{Error, Relation, RelationType, util::escape_rust_keyword};

impl Relation {
    /// Every Rust identifier the writer derives from the relation's DB
    /// names: the module and variant of the entity it reaches and the
    /// `Column` variants it pairs.
    // [spec:pgorm:sem:codegen.entity.keywords+2]
    // [spec:pgorm:sem:codegen.entity.relations+5]
    pub(crate) fn validate_idents(&self) -> Result<(), Error> {
        let context = format!("relation to `{}`", self.ref_ident());
        safe_ident(
            &context,
            &escape_rust_keyword(self.ref_table.to_snake_case()),
        )?;
        safe_ident(&context, &self.ref_table.to_upper_camel_case())?;
        let period = self
            .period
            .iter()
            .flat_map(|(column, ref_column)| [column, ref_column]);
        for column in self
            .columns
            .iter()
            .chain(self.ref_columns.iter())
            .chain(period)
        {
            safe_ident(&context, &column.to_upper_camel_case())?;
        }
        Ok(())
    }

    // [spec:pgorm:sem:codegen.entity.relations+5]
    pub fn get_enum_name(&self) -> Ident {
        let name = if self.self_referencing {
            format_ident!("SelfRef")
        } else {
            format_ident!("{}", self.ref_table.to_upper_camel_case())
        };
        if self.num_suffix > 0 {
            format_ident!("{}{}", name, self.num_suffix)
        } else {
            name
        }
    }

    pub fn get_module_name(&self) -> Option<Ident> {
        self.module_name().map(|module| format_ident!("{}", module))
    }

    /// The variant's `#[pgorm(..)]`, every value but the key's name Rust
    /// written bare: the entity reached, the column pairs, the period, the
    /// actions and the key check.
    // [spec:pgorm:sem:codegen.entity.relations+5]
    // [spec:pgorm:req:macros.derive.code-values+4]
    pub fn get_attrs(&self) -> TokenStream {
        let rel_type = self.get_rel_type();
        let module = self
            .get_module_name()
            .map(|module_name| quote! { super::#module_name:: });
        let ref_entity = quote! { #module Entity };
        match self.rel_type {
            RelationType::HasOne | RelationType::HasMany => {
                let unrelated = self.unrelated().then(|| quote! { , related = false });
                quote! {
                    #[pgorm(#rel_type = #ref_entity #unrelated)]
                }
            }
            RelationType::BelongsTo => {
                let from = Self::column_list(&self.get_column_camel_case(), None);
                let to = Self::column_list(&self.get_ref_column_camel_case(), module.as_ref());
                let mut parts = vec![
                    quote! { #rel_type = #ref_entity },
                    quote! { from = #from },
                    quote! { to = #to },
                ];
                if let Some((src_column, ref_column)) = &self.period {
                    let from_period = format_ident!("{}", src_column.to_upper_camel_case());
                    let to_period = format_ident!("{}", ref_column.to_upper_camel_case());
                    parts.push(quote! { from_period = Column::#from_period });
                    parts.push(quote! { to_period = #module Column::#to_period });
                }
                if let Some(action) = &self.on_update {
                    let action = Self::checked_foreign_key_action(action);
                    parts.push(quote! { on_update = ForeignKeyAction::#action });
                }
                if let Some(action) = &self.on_delete {
                    let action = Self::checked_foreign_key_action(action);
                    parts.push(quote! { on_delete = ForeignKeyAction::#action });
                }
                parts.extend(self.get_key_attrs());
                if self.unrelated() {
                    parts.push(quote! { related = false });
                }
                quote! {
                    #[pgorm(#(#parts),*)]
                }
            }
        }
    }

    /// Whether the variant says `related = false`: the entity it reaches is
    /// one a junction's `Related` impl reaches too, and the derive would
    /// otherwise write a second. An entity two variants reach gets no impl
    /// from the derive, so only a variant alone in reaching one says it.
    // [spec:pgorm:sem:codegen.entity.relations.related+2]
    // [spec:pgorm:sem:macros.derive.boilerplate]
    fn unrelated(&self) -> bool {
        !self.impl_related && self.num_suffix == 0
    }

    /// The `Column` variants a side of the key names, under `module`: the
    /// variant alone for a one-column key, a tuple of them for a composite one.
    fn column_list(columns: &[Ident], module: Option<&TokenStream>) -> TokenStream {
        let paths: Vec<TokenStream> = columns
            .iter()
            .map(|column| quote! { #module Column::#column })
            .collect();
        match paths.as_slice() {
            [one] => one.clone(),
            _ => quote! { (#(#paths),*) },
        }
    }

    /// The `deferrability` and `enforcement` attribute keys, where the key
    /// declared them.
    // [spec:pgorm:sem:codegen.entity.relations+5]
    fn get_key_attrs(&self) -> Vec<TokenStream> {
        let mut attrs = Vec::new();
        if let Some(deferrability) = self.deferrability {
            let variant = format_ident!("{}", Self::get_deferrability(deferrability));
            attrs.push(quote! { deferrability = Deferrability::#variant });
        }
        if let Some(enforcement) = self.enforcement {
            let variant = format_ident!("{}", Self::get_enforcement(enforcement));
            attrs.push(quote! { enforcement = Enforcement::#variant });
        }
        attrs
    }

    fn get_deferrability(deferrability: Deferrability) -> &'static str {
        match deferrability {
            Deferrability::NotDeferrable => "NotDeferrable",
            Deferrability::DeferrableInitiallyImmediate => "DeferrableInitiallyImmediate",
            Deferrability::DeferrableInitiallyDeferred => "DeferrableInitiallyDeferred",
        }
    }

    fn get_enforcement(enforcement: Enforcement) -> &'static str {
        match enforcement {
            Enforcement::Enforced => "Enforced",
            Enforcement::NotEnforced => "NotEnforced",
        }
    }

    pub fn get_rel_type(&self) -> Ident {
        match self.rel_type {
            RelationType::HasOne => format_ident!("has_one"),
            RelationType::HasMany => format_ident!("has_many"),
            RelationType::BelongsTo => format_ident!("belongs_to"),
        }
    }

    pub fn get_column_camel_case(&self) -> Vec<Ident> {
        self.columns
            .iter()
            .map(|col| format_ident!("{}", col.to_upper_camel_case()))
            .collect()
    }

    pub fn get_ref_column_camel_case(&self) -> Vec<Ident> {
        self.ref_columns
            .iter()
            .map(|col| format_ident!("{}", col.to_upper_camel_case()))
            .collect()
    }

    /// [`get_foreign_key_action`](Self::get_foreign_key_action) of an action
    /// a validated relation holds, as the variant's identifier.
    fn checked_foreign_key_action(action: &ForeignKeyAction) -> Ident {
        let action = Self::get_foreign_key_action(action).unwrap_or_else(|| {
            unreachable!(
                "foreign key action {action:?} reached the writer; \
                 every relation is validated before it is written"
            )
        });
        format_ident!("{action}")
    }
}

#[cfg(test)]
mod tests {
    use crate::{Relation, RelationType};
    use pgorm_query::ForeignKeyAction;

    fn setup() -> Vec<Relation> {
        vec![
            Relation {
                ref_table: "fruit".to_owned(),
                ref_schema: None,
                columns: vec!["id".to_owned()],
                ref_columns: vec!["cake_id".to_owned()],
                rel_type: RelationType::HasOne,
                on_delete: None,
                on_update: None,
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
                columns: vec!["filling_id".to_owned()],
                ref_columns: vec!["id".to_owned()],
                rel_type: RelationType::BelongsTo,
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
                columns: vec!["filling_id".to_owned()],
                ref_columns: vec!["id".to_owned()],
                rel_type: RelationType::HasMany,
                on_delete: Some(ForeignKeyAction::Cascade),
                on_update: None,
                enforcement: None,
                deferrability: None,
                period: None,
                self_referencing: false,
                num_suffix: 0,
                impl_related: true,
            },
        ]
    }

    #[test]
    fn test_get_module_name() {
        let relations = setup();
        let snake_cases = vec!["fruit", "filling", "filling"];
        for (rel, snake_case) in relations.into_iter().zip(snake_cases) {
            assert_eq!(rel.get_module_name().unwrap().to_string(), snake_case);
        }
    }

    #[test]
    fn test_get_enum_name() {
        let relations = setup();
        let camel_cases = vec!["Fruit", "Filling", "Filling"];
        for (rel, camel_case) in relations.into_iter().zip(camel_cases) {
            assert_eq!(rel.get_enum_name().to_string(), camel_case);
        }
    }

    #[test]
    fn test_get_rel_type() {
        let relations = setup();
        let rel_types = vec!["has_one", "belongs_to", "has_many"];
        for (rel, rel_type) in relations.into_iter().zip(rel_types) {
            assert_eq!(rel.get_rel_type(), rel_type);
        }
    }

    #[test]
    fn test_get_column_camel_case() {
        let relations = setup();
        let cols = vec!["Id", "FillingId", "FillingId"];
        for (rel, col) in relations.into_iter().zip(cols) {
            assert_eq!(rel.get_column_camel_case(), [col]);
        }
    }

    #[test]
    fn test_get_ref_column_camel_case() {
        let relations = setup();
        let ref_cols = vec!["CakeId", "Id", "Id"];
        for (rel, ref_col) in relations.into_iter().zip(ref_cols) {
            assert_eq!(rel.get_ref_column_camel_case(), [ref_col]);
        }
    }
}
