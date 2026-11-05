use heck::ToUpperCamelCase;
use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use syn::Expr;
use syn::spanned::Spanned;

use super::from_query_result::{Decoder, expand_from_query_result};
use super::util::{bare_code, parse_derived_ident, unknown_pgorm_key};

#[derive(Debug)]
enum Error {
    InputNotStruct,
    EntityNotSpecific,
    NotSupportGeneric(Span),
    BothFromColAndFromExpr(Span),
    Syn(syn::Error),
}
#[derive(Debug, PartialEq, Eq)]
enum ColumnAs {
    /// column in the model
    Col { entity: syn::Type, col: syn::Ident },
    /// a column the field names, aliased to the field
    ColAlias { col: syn::ExprPath, field: String },
    /// from an expr
    Expr { expr: syn::Expr, field_name: String },
}

impl ColumnAs {
    /// The entity column the field reads, whose read cast the field's
    /// reported column carries, or `None` for an expression.
    // [spec:pgorm:sem:macros.derive.partial-model+6]
    fn column(&self) -> Option<TokenStream> {
        match self {
            ColumnAs::Col { entity, col } => {
                Some(quote!(<#entity as pgorm::EntityTrait>::Column::#col))
            }
            ColumnAs::ColAlias { col, .. } => Some(quote!(#col)),
            ColumnAs::Expr { .. } => None,
        }
    }
}

struct PartialModel {
    ident: syn::Ident,
    fields: Vec<ColumnAs>,
    /// The struct's `FromQueryResult`, which the derive writes beside the
    /// projection.
    decode: TokenStream,
}

impl PartialModel {
    fn new(input: syn::DeriveInput) -> Result<Self, Error> {
        if !input.generics.params.is_empty() {
            return Err(Error::NotSupportGeneric(input.generics.params.span()));
        }

        let data = input.data.clone();
        let syn::Data::Struct(
            syn::DataStruct {
                fields: syn::Fields::Named(syn::FieldsNamed { named: fields, .. }),
                ..
            },
            ..,
        ) = input.data
        else {
            return Err(Error::InputNotStruct);
        };

        let mut entity: Option<syn::Type> = None;

        for attr in input.attrs.iter() {
            if !attr.path().is_ident("pgorm") {
                continue;
            }

            attr.parse_nested_meta(|meta| {
                if !meta.path.is_ident("entity") {
                    return Err(unknown_pgorm_key(&meta));
                }
                if entity.is_some() {
                    return Err(meta.error("duplicate `entity`"));
                }
                entity = Some(bare_code(&meta, "entity")?);
                Ok(())
            })
            .map_err(Error::Syn)?;
        }

        let mut column_as_list = Vec::with_capacity(fields.len());

        for field in fields {
            let field_span = field.span();
            let field_name = field.ident.ok_or(Error::InputNotStruct)?;

            // Both trackers accumulate across every meta item of every `#[pgorm(...)]`
            // attribute on the field, so a second key cannot erase the first: a repeat of
            // one key is a duplicate, and one of each is the conflict guarded below.
            let mut from_col: Option<syn::ExprPath> = None;
            let mut from_expr: Option<Expr> = None;

            for attr in field.attrs.iter() {
                if !attr.path().is_ident("pgorm") {
                    continue;
                }

                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("from_col") {
                        if from_col.is_some() {
                            return Err(meta.error("duplicate `from_col`"));
                        }
                        from_col = Some(bare_code(&meta, "from_col")?);
                        Ok(())
                    } else if meta.path.is_ident("from_expr") {
                        if from_expr.is_some() {
                            return Err(meta.error("duplicate `from_expr`"));
                        }
                        from_expr = Some(bare_code(&meta, "from_expr")?);
                        Ok(())
                    } else if meta.path.is_ident("skip") {
                        Ok(())
                    } else {
                        Err(unknown_pgorm_key(&meta))
                    }
                })
                .map_err(Error::Syn)?;
            }

            let col_as = match (from_col, from_expr) {
                (None, None) => ColumnAs::Col {
                    entity: entity.clone().ok_or(Error::EntityNotSpecific)?,
                    col: parse_derived_ident(
                        &field_name.to_string().to_upper_camel_case(),
                        &field_name,
                    )
                    .map_err(Error::Syn)?,
                },
                (None, Some(expr)) => ColumnAs::Expr {
                    expr,
                    field_name: field_name.to_string(),
                },
                (Some(col), None) => ColumnAs::ColAlias {
                    col,
                    field: field_name.to_string(),
                },
                (Some(_), Some(_)) => return Err(Error::BothFromColAndFromExpr(field_span)),
            };
            column_as_list.push(col_as);
        }

        let columns: Vec<Option<TokenStream>> =
            column_as_list.iter().map(ColumnAs::column).collect();
        let decode = expand_from_query_result(
            input.ident.clone(),
            data,
            input.generics,
            Decoder::PartialModel(&columns),
        )
        .map_err(Error::Syn)?;

        Ok(Self {
            ident: input.ident,
            fields: column_as_list,
            decode,
        })
    }

    fn expand(&self) -> syn::Result<TokenStream> {
        let mut expanded = self.impl_partial_model_trait();
        expanded.extend(self.decode.clone());
        Ok(expanded)
    }

    fn impl_partial_model_trait(&self) -> TokenStream {
        let select_ident = format_ident!("select");
        let PartialModel { ident, fields, .. } = self;
        let select_col_code_gen = fields.iter().map(|col_as| match col_as {
            ColumnAs::Col { entity, col: ident } => {
                let col_value = quote!( <#entity as pgorm::EntityTrait>::Column:: #ident);
                quote!(let #select_ident =  pgorm::QuerySelect::column(#select_ident, #col_value);)
            },
            ColumnAs::ColAlias { col, field } => {
                quote!(let #select_ident =  pgorm::QuerySelect::column_as(#select_ident, pgorm::ColumnTrait::select_as(&#col, pgorm::ColumnTrait::into_expr(#col)), pgorm::pgorm_query::alias(#field));)
            },
            ColumnAs::Expr { expr, field_name } => {
                quote!(let #select_ident =  pgorm::QuerySelect::column_as(#select_ident, #expr, pgorm::pgorm_query::alias(#field_name));)
            },
        });

        quote! {
            #[automatically_derived]
            impl pgorm::PartialModelTrait for #ident{
                fn select_cols<S: pgorm::QuerySelect>(#select_ident: S) -> S::Projected {
                    #(#select_col_code_gen)*
                    #select_ident
                }
            }
        }
    }
}

// [spec:pgorm:sem:macros.derive.partial-model+6]
// [spec:pgorm:req:macros.derive.code-values+3]
pub fn expand_derive_partial_model(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let ident_span = input.ident.span();

    match PartialModel::new(input) {
        Ok(partial_model) => partial_model.expand(),
        Err(Error::NotSupportGeneric(span)) => Ok(quote_spanned! {
            span => compile_error!("you can only derive `PartialModel` on named struct");
        }),
        Err(Error::BothFromColAndFromExpr(span)) => Ok(quote_spanned! {
            span => compile_error!("you can only use one of `from_col` or `from_expr`");
        }),
        Err(Error::EntityNotSpecific) => Ok(quote_spanned! {
            ident_span => compile_error!("you need specific which entity you are using")
        }),
        Err(Error::InputNotStruct) => Ok(quote_spanned! {
            ident_span => compile_error!("you can only derive `PartialModel` on named struct");
        }),
        Err(Error::Syn(err)) => Err(err),
    }
}

#[cfg(test)]
mod test {
    use quote::format_ident;
    use syn::{DeriveInput, Type, parse_str};

    use crate::derives::partial_model::ColumnAs;

    use super::PartialModel;

    #[cfg(test)]
    type StdResult<T> = Result<T, Box<dyn std::error::Error>>;

    #[cfg(test)]
    const CODE_SNIPPET: &str = r#"
#[pgorm(entity = Entity)]
struct PartialModel{
    default_field: i32,
    #[pgorm(from_col = Column::Bar)]
    alias_field: i32,
    #[pgorm(from_expr = Expr::val(1).add(1))]
    expr_field : i32
}
"#;
    // [spec:pgorm:sem:macros.derive.partial-model+6/test]
    #[test]
    fn test_load_macro_input() -> StdResult<()> {
        let input = parse_str::<DeriveInput>(CODE_SNIPPET)?;

        let middle = PartialModel::new(input).unwrap();
        assert_eq!(middle.ident, format_ident!("PartialModel"));
        assert_eq!(middle.fields.len(), 3);
        assert_eq!(
            middle.fields[0],
            ColumnAs::Col {
                entity: parse_str::<Type>("Entity").unwrap(),
                col: format_ident!("DefaultField")
            }
        );
        assert_eq!(
            middle.fields[1],
            ColumnAs::ColAlias {
                col: parse_str("Column::Bar").unwrap(),
                field: "alias_field".to_string()
            },
        );
        assert_eq!(
            middle.fields[2],
            ColumnAs::Expr {
                expr: syn::parse_str("Expr::val(1).add(1)").unwrap(),
                field_name: "expr_field".to_string()
            }
        );

        Ok(())
    }
}
