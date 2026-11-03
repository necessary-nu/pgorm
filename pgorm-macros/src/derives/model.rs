use super::util::{
    COMPOSITE_KEYS, Reader, bare_code, field_not_ignored, field_variant, spell_type,
    unknown_pgorm_key,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use std::iter::FromIterator;
use syn::Ident;

/// The entity a further model belongs to, from its container's `entity`:
/// the one key `Model` reads there. Under the composite the entity is
/// always `Entity`, and the composite reads the container itself.
// [spec:pgorm:sem:macros.derive.model+6]
// [spec:pgorm:req:macros.derive.code-values+3]
fn model_entity(attrs: &[syn::Attribute], reader: Reader) -> syn::Result<Option<Ident>> {
    let mut entity = None;
    if reader == Reader::Composite {
        return Ok(entity);
    }
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("pgorm")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("entity") {
                if entity.is_some() {
                    return Err(meta.error("duplicate `entity`"));
                }
                entity = Some(bare_code(&meta, "entity")?);
                return Ok(());
            }
            match meta.path.get_ident() {
                Some(key) if COMPOSITE_KEYS.contains(&key.to_string().as_str()) => {
                    Err(meta.error(format!(
                        "`{key}` is the composite derive's: it goes on the entity's `Model` \
                         under `Entity`; `Model` reads `entity`"
                    )))
                }
                _ => Err(unknown_pgorm_key(&meta)),
            }
        })?;
    }
    Ok(entity)
}

enum Error {
    InputNotStruct,
    Syn(syn::Error),
}

struct Model {
    column_idents: Vec<syn::Ident>,
    entity_ident: syn::Ident,
    field_idents: Vec<syn::Ident>,
    field_types: Vec<syn::Type>,
    ident: syn::Ident,
    ignore_attrs: Vec<bool>,
}

impl Model {
    fn new(input: syn::DeriveInput, reader: Reader) -> Result<Self, Error> {
        let fields = match input.data {
            syn::Data::Struct(syn::DataStruct {
                fields: syn::Fields::Named(syn::FieldsNamed { named, .. }),
                ..
            }) => named,
            _ => return Err(Error::InputNotStruct),
        };

        let ident = input.ident;
        let entity_ident = model_entity(&input.attrs, reader)
            .map_err(Error::Syn)?
            .unwrap_or_else(|| format_ident!("Entity"));

        let field_idents = fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap().clone())
            .collect();

        let field_types = fields.iter().map(|field| field.ty.clone()).collect();

        let column_idents = fields
            .iter()
            .map(|field| field_variant(field, reader))
            .collect::<syn::Result<_>>()
            .map_err(Error::Syn)?;

        let ignore_attrs = fields
            .iter()
            .map(|field| !field_not_ignored(field))
            .collect();

        Ok(Model {
            column_idents,
            entity_ident,
            field_idents,
            field_types,
            ident,
            ignore_attrs,
        })
    }

    fn expand(&self) -> syn::Result<TokenStream> {
        let expanded_impl_from_query_result = self.impl_from_query_result();
        let expanded_impl_model_trait = self.impl_model_trait();
        let expanded_impl_column_fields = self.impl_column_fields();

        Ok(TokenStream::from_iter([
            expanded_impl_from_query_result,
            expanded_impl_model_trait,
            expanded_impl_column_fields,
        ]))
    }

    /// `ColumnField` for each field reading a column, keyed by the column's
    /// discriminant: the field's type reachable from a column named as a
    /// value, which `OneOf` holds to its target's key.
    // [spec:pgorm:sem:macros.derive.model+6]
    // [spec:pgorm:def:disjoint.one-of+1]
    fn impl_column_fields(&self) -> TokenStream {
        let ident = &self.ident;
        let entity_ident = &self.entity_ident;
        let fields = self
            .field_idents
            .iter()
            .zip(&self.field_types)
            .zip(&self.column_idents)
            .zip(&self.ignore_attrs)
            .filter(|(_, ignore)| !**ignore)
            .map(|(((field, field_type), column), _)| {
                quote! {
                    #[automatically_derived]
                    impl pgorm::ColumnField<{ <#entity_ident as pgorm::entity::EntityTrait>::Column::#column as usize }> for #ident {
                        type Value = #field_type;

                        fn field(&self) -> &#field_type {
                            &self.#field
                        }
                    }
                }
            });
        quote!(#(#fields)*)
    }

    fn impl_from_query_result(&self) -> TokenStream {
        let ident = &self.ident;
        let field_idents = &self.field_idents;
        let column_idents = &self.column_idents;
        let field_values: Vec<TokenStream> = column_idents
            .iter()
            .zip(&self.ignore_attrs)
            .map(|(column_ident, ignore)| {
                if *ignore {
                    quote! {
                        Default::default()
                    }
                } else {
                    quote! {
                        row.try_get(pre, pgorm::StaticName::as_str(&<<Self as pgorm::ModelTrait>::Entity as pgorm::entity::EntityTrait>::Column::#column_ident).into())?
                    }
                }
            })
            .collect();

        // [spec:pgorm:sem:macros.derive.model+6]    the reflected column shape
        let expected_columns: Vec<TokenStream> = column_idents
            .iter()
            .zip(&self.field_types)
            .zip(&self.ignore_attrs)
            .filter(|(_, ignore)| !**ignore)
            .map(|((column_ident, field_type), _)| {
                let rust_type = spell_type(field_type);
                quote! {
                    pgorm::ExpectedColumn::new(
                        pgorm::StaticName::as_str(&<<Self as pgorm::ModelTrait>::Entity as pgorm::entity::EntityTrait>::Column::#column_ident),
                        #rust_type,
                        <#field_type as pgorm::TryGetable>::accepts,
                    )
                }
            })
            .collect();

        quote!(
            #[automatically_derived]
            impl pgorm::FromQueryResult for #ident {
                fn from_query_result(row: &pgorm::QueryResult, pre: &str) -> std::result::Result<Self, pgorm::Error> {
                    Ok(Self {
                        #(#field_idents: #field_values),*
                    })
                }

                fn expected_columns() -> std::option::Option<std::vec::Vec<pgorm::ExpectedColumn>> {
                    std::option::Option::Some(std::vec![
                        #(#expected_columns),*
                    ])
                }
            }
        )
    }

    fn impl_model_trait<'a>(&'a self) -> TokenStream {
        let ident = &self.ident;
        let entity_ident = &self.entity_ident;
        let ignore_attrs = &self.ignore_attrs;
        let ignore = |(ident, ignore): (&'a Ident, &bool)| -> Option<&'a Ident> {
            if *ignore { None } else { Some(ident) }
        };
        let field_idents: Vec<&Ident> = self
            .field_idents
            .iter()
            .zip(ignore_attrs)
            .filter_map(ignore)
            .collect();
        let column_idents: Vec<&Ident> = self
            .column_idents
            .iter()
            .zip(ignore_attrs)
            .filter_map(ignore)
            .collect();

        let missing_field_msg = format!("field does not exist on {ident}");
        let mismatch_msgs: Vec<String> = field_idents
            .iter()
            .map(|field| format!("value does not match the type of {ident} field {field}"))
            .collect();

        quote!(
            #[automatically_derived]
            impl pgorm::ModelTrait for #ident {
                type Entity = #entity_ident;

                fn get(&self, c: <Self::Entity as pgorm::entity::EntityTrait>::Column) -> pgorm::Value {
                    match c {
                        #(<Self::Entity as pgorm::entity::EntityTrait>::Column::#column_idents => self.#field_idents.clone().into(),)*
                        _ => panic!(#missing_field_msg),
                    }
                }

                fn set(&mut self, c: <Self::Entity as pgorm::entity::EntityTrait>::Column, v: pgorm::Value) -> std::result::Result<(), pgorm::Error> {
                    match c {
                        #(<Self::Entity as pgorm::entity::EntityTrait>::Column::#column_idents => {
                            self.#field_idents = pgorm::pgorm_query::ValueType::try_from(v)
                                .map_err(|_| pgorm::Error::Type(#mismatch_msgs.to_owned()))?;
                            Ok(())
                        },)*
                        _ => Err(pgorm::Error::Type(#missing_field_msg.to_owned())),
                    }
                }
            }
        )
    }
}

/// `ModelTrait` and `FromQueryResult` for a model, as `reader` reads its
/// keys: every key under the composite, which validated them, and only
/// `entity`, `variant` and `ignore` under `Model` itself.
// [spec:pgorm:sem:macros.derive.model+6]
// [spec:pgorm:syn:macros.derive.entity-model.attrs+9]    the field-key vocabulary this
// derive shares with the `Entity` derive
pub fn expand_derive_model(input: syn::DeriveInput, reader: Reader) -> syn::Result<TokenStream> {
    let ident_span = input.ident.span();
    match Model::new(input, reader) {
        Ok(model) => model.expand(),
        Err(Error::InputNotStruct) => Ok(quote_spanned! {
            ident_span => compile_error!("you can only derive `Model` on structs");
        }),
        Err(Error::Syn(err)) => Err(err),
    }
}
