use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{Deferrability, Enforcement, ForeignKeyAction, TableForeignKey};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::{
    Error, TableIdent,
    util::{escape_rust_keyword, safe_ident},
};

#[derive(Clone, Debug)]
pub enum RelationType {
    HasOne,
    HasMany,
    BelongsTo,
}

#[derive(Clone, Debug)]
pub struct Relation {
    pub(crate) ref_table: String,
    /// The schema qualifying the referenced table, once the relation has been
    /// resolved against the schema's own tables. Never reaches the generated
    /// text: a module path is the bare name, which is one table's alone.
    pub(crate) ref_schema: Option<String>,
    pub(crate) columns: Vec<String>,
    pub(crate) ref_columns: Vec<String>,
    pub(crate) rel_type: RelationType,
    pub(crate) on_update: Option<ForeignKeyAction>,
    pub(crate) on_delete: Option<ForeignKeyAction>,
    /// `NotEnforced` when the key is; an explicit `ENFORCED` states the
    /// default and reads as nothing said.
    pub(crate) enforcement: Option<Enforcement>,
    /// When the key is checked, if it is deferrable; an explicit `NOT
    /// DEFERRABLE` states the default and reads as nothing said.
    pub(crate) deferrability: Option<Deferrability>,
    /// The `(column, referenced column)` pair a temporal foreign key matches
    /// as periods, after its pairs.
    // [spec:pgorm:sem:codegen.entity.relations+4]
    pub(crate) period: Option<(String, String)>,
    pub(crate) self_referencing: bool,
    pub(crate) num_suffix: usize,
    pub(crate) impl_related: bool,
}

impl Relation {
    /// Which table this relation references.
    // [spec:pgorm:sem:codegen.entity.transform+16]
    pub fn ref_ident(&self) -> TableIdent {
        TableIdent {
            table: self.ref_table.clone(),
            schema: self.ref_schema.clone(),
        }
    }

    // [spec:pgorm:sem:codegen.entity.keywords+2]
    // [spec:pgorm:sem:codegen.entity.relations+4]
    pub(crate) fn validate(&self) -> Result<(), Error> {
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
        for action in self.on_update.iter().chain(&self.on_delete) {
            if Self::get_foreign_key_action(action).is_none() {
                return Err(Error::TransformError(format!(
                    "{context}: foreign key action {action:?} is not supported by codegen"
                )));
            }
        }
        if self.columns.len() != self.ref_columns.len() {
            return Err(Error::TransformError(format!(
                "{context}: constrains {} column(s) against {} referenced column(s); a relation \
                 joins its columns in pairs",
                self.columns.len(),
                self.ref_columns.len(),
            )));
        }
        Ok(())
    }

    // [spec:pgorm:sem:codegen.entity.relations+4]
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
        if self.self_referencing {
            None
        } else {
            Some(format_ident!(
                "{}",
                escape_rust_keyword(self.ref_table.to_snake_case())
            ))
        }
    }

    // [spec:pgorm:sem:codegen.entity.relations+4]
    pub fn get_attrs(&self) -> TokenStream {
        let rel_type = self.get_rel_type();
        let module_name = if let Some(module_name) = self.get_module_name() {
            format!("super::{module_name}::")
        } else {
            String::new()
        };
        let ref_entity = format!("{module_name}Entity");
        match self.rel_type {
            RelationType::HasOne | RelationType::HasMany => {
                quote! {
                    #[pgorm(#rel_type = #ref_entity)]
                }
            }
            RelationType::BelongsTo => {
                let map_src_column = |src_column: &Ident| format!("Column::{src_column}");
                let map_ref_column =
                    |ref_column: &Ident| format!("{module_name}Column::{ref_column}");
                let map_punctuated = |punctuated: Vec<String>| {
                    let len = punctuated.len();
                    let punctuated = punctuated.join(", ");
                    match len {
                        0..=1 => punctuated,
                        _ => format!("({})", punctuated),
                    }
                };
                let (from, to) =
                    self.get_src_ref_columns(map_src_column, map_ref_column, map_punctuated);

                let on_update = if let Some(action) = &self.on_update {
                    let action = Self::checked_foreign_key_action(action);
                    quote! {
                        on_update = #action,
                    }
                } else {
                    quote! {}
                };
                let on_delete = if let Some(action) = &self.on_delete {
                    let action = Self::checked_foreign_key_action(action);
                    quote! {
                        on_delete = #action,
                    }
                } else {
                    quote! {}
                };
                let key_check = self.get_key_attrs();
                let period = match &self.period {
                    Some((src_column, ref_column)) => {
                        let from_period = format!("Column::{}", src_column.to_upper_camel_case());
                        let to_period =
                            format!("{module_name}Column::{}", ref_column.to_upper_camel_case());
                        quote! { from_period = #from_period, to_period = #to_period, }
                    }
                    None => quote! {},
                };
                quote! {
                    #[pgorm(
                        #rel_type = #ref_entity,
                        from = #from,
                        to = #to,
                        #period
                        #on_update
                        #on_delete
                        #key_check
                    )]
                }
            }
        }
    }

    /// The `deferrability = ".."` and `enforcement = ".."` attribute keys,
    /// where the key declared them.
    // [spec:pgorm:sem:codegen.entity.relations+4]
    fn get_key_attrs(&self) -> TokenStream {
        let mut attrs = TokenStream::new();
        if let Some(deferrability) = self.deferrability {
            let variant = Self::get_deferrability(deferrability);
            attrs.extend(quote! { deferrability = #variant, });
        }
        if let Some(enforcement) = self.enforcement {
            let variant = Self::get_enforcement(enforcement);
            attrs.extend(quote! { enforcement = #variant, });
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

    /// The `ForeignKeyAction` variant naming `action`, as generated source
    /// spells it, or `None` for an action codegen has no spelling of, which
    /// [`validate`](Self::validate) refuses.
    pub fn get_foreign_key_action(action: &ForeignKeyAction) -> Option<String> {
        let action = match action {
            ForeignKeyAction::Restrict => "Restrict",
            ForeignKeyAction::Cascade => "Cascade",
            ForeignKeyAction::SetNull => "SetNull",
            ForeignKeyAction::NoAction => "NoAction",
            ForeignKeyAction::SetDefault => "SetDefault",
            _ => return None,
        };
        Some(action.to_owned())
    }

    /// [`get_foreign_key_action`](Self::get_foreign_key_action) of an action
    /// a validated relation holds.
    fn checked_foreign_key_action(action: &ForeignKeyAction) -> String {
        Self::get_foreign_key_action(action).unwrap_or_else(|| {
            unreachable!(
                "foreign key action {action:?} reached the writer; \
                 every relation is validated before it is written"
            )
        })
    }

    pub fn get_src_ref_columns<F1, F2, F3, T, I>(
        &self,
        map_src_column: F1,
        map_ref_column: F2,
        map_punctuated: F3,
    ) -> (T, T)
    where
        F1: Fn(&Ident) -> T,
        F2: Fn(&Ident) -> T,
        F3: Fn(I) -> T,
        I: Extend<T> + Default,
    {
        let from: I =
            self.get_column_camel_case()
                .iter()
                .fold(I::default(), |mut acc, src_column| {
                    acc.extend([map_src_column(src_column)]);
                    acc
                });
        let to: I =
            self.get_ref_column_camel_case()
                .iter()
                .fold(I::default(), |mut acc, ref_column| {
                    acc.extend([map_ref_column(ref_column)]);
                    acc
                });

        (map_punctuated(from), map_punctuated(to))
    }
}

// [spec:pgorm:sem:codegen.entity.transform+16]
impl From<&TableForeignKey> for Relation {
    fn from(tbl_fk: &TableForeignKey) -> Self {
        let ref_table = TableIdent::of(tbl_fk.get_ref_table());
        Self {
            ref_table: ref_table.table,
            ref_schema: ref_table.schema,
            columns: tbl_fk.get_columns(),
            ref_columns: tbl_fk.get_ref_columns(),
            rel_type: RelationType::BelongsTo,
            on_delete: tbl_fk.get_on_delete(),
            on_update: tbl_fk.get_on_update(),
            enforcement: tbl_fk
                .get_enforcement()
                .filter(|enforcement| *enforcement == Enforcement::NotEnforced),
            deferrability: tbl_fk
                .get_deferrability()
                .filter(|deferrability| *deferrability != Deferrability::NotDeferrable),
            period: tbl_fk
                .get_period()
                .map(|(column, ref_column)| (column.to_string(), ref_column.to_string())),
            self_referencing: false,
            num_suffix: 0,
            impl_related: true,
        }
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
