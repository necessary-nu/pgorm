use proc_macro2::TokenStream;
use quote::quote;

struct DeriveMigrationName {
    ident: syn::Ident,
}

impl DeriveMigrationName {
    fn new(input: syn::DeriveInput) -> Self {
        let ident = input.ident;

        DeriveMigrationName { ident }
    }

    fn expand(&self) -> TokenStream {
        let ident = &self.ident;

        quote!(
            #[automatically_derived]
            impl pgorm_migration::MigrationName for #ident {
                fn name(&self) -> &str {
                    pgorm_migration::util::get_file_stem(file!())
                }
            }
        )
    }
}

/// Method to derive a MigrationName
// [spec:pgorm:def:macros.derive+4]
pub fn expand_derive_migration_name(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    Ok(DeriveMigrationName::new(input).expand())
}
