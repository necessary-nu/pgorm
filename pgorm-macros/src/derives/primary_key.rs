use proc_macro2::{Ident, TokenStream};
use quote::quote;

/// The impls the composite derive writes for the `PrimaryKey` enum it
/// generates, one variant per key column: its name, and the `Column` variant
/// of the same name it maps to and from. `keys` pairs each variant with its
/// column's SQL name.
// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7]
pub fn expand_primary_key(keys: &[(Ident, String)]) -> TokenStream {
    let variant: Vec<&Ident> = keys.iter().map(|(variant, _)| variant).collect();
    let name: Vec<&String> = keys.iter().map(|(_, name)| name).collect();

    quote!(
        #[automatically_derived]
        impl pgorm::SqlName for PrimaryKey {
            fn unquoted(&self, s: &mut dyn std::fmt::Write) {
                write!(s, "{}", pgorm::StaticName::as_str(self)).unwrap();
            }
        }

        #[automatically_derived]
        impl pgorm::StaticName for PrimaryKey {
            fn as_str(&self) -> &str {
                match self {
                    #(Self::#variant => #name),*
                }
            }
        }

        #[automatically_derived]
        impl pgorm::PrimaryKeyToColumn for PrimaryKey {
            type Column = Column;

            fn into_column(self) -> Self::Column {
                match self {
                    #(Self::#variant => Self::Column::#variant,)*
                }
            }

            fn from_column(col: Self::Column) -> Option<Self> {
                match col {
                    #(Self::Column::#variant => Some(Self::#variant),)*
                    _ => None,
                }
            }
        }
    )
}
