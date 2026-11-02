//! Iteration over an enum's variants, adapted from strum's `EnumIter`
//! (<https://github.com/Peternator7/strum>, MIT; see `STRUM-LICENSE`).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Variant, Visibility};

/// `pgorm::Iterable` for the enum `ident`, over `variants` in the order given,
/// and the `{ident}Iter` iterator it yields. `Relation`, `Column`,
/// `ActiveEnum` and the composite derive write it for the enums they are
/// given or make, so no program derives iteration itself. Each of those enums
/// is of unit variants.
// [spec:pgorm:sem:macros.derive.boilerplate]
// [spec:pgorm:sem:macros.derive.enum-iter+1]
pub(crate) fn impl_iterable<'a>(
    ident: &Ident,
    vis: &Visibility,
    variants: impl IntoIterator<Item = &'a Variant>,
) -> TokenStream {
    let doc_comment = format!("An iterator over the variants of [{ident}]");

    let mut arms = Vec::new();
    for (idx, variant) in variants.into_iter().enumerate() {
        let variant_ident = &variant.ident;
        arms.push(quote! {#idx => ::core::option::Option::Some(#ident::#variant_ident)});
    }

    let variant_count = arms.len();
    arms.push(quote! { _ => ::core::option::Option::None });
    let iter_name = format_ident!("{}Iter", ident);
    let iter_name_debug = iter_name.to_string();

    quote! {
        #[doc = #doc_comment]
        #[allow(missing_copy_implementations)]
        #vis struct #iter_name {
            idx: usize,
            back_idx: usize,
        }

        #[automatically_derived]
        impl ::core::fmt::Debug for #iter_name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.debug_struct(#iter_name_debug)
                    .field("len", &self.len())
                    .finish()
            }
        }

        #[automatically_derived]
        impl #iter_name {
            fn get(&self, idx: usize) -> ::core::option::Option<#ident> {
                match idx {
                    #(#arms),*
                }
            }
        }

        #[automatically_derived]
        impl pgorm::Iterable for #ident {
            type Iterator = #iter_name;
            fn iter() -> #iter_name {
                #iter_name {
                    idx: 0,
                    back_idx: 0,
                }
            }
        }

        #[automatically_derived]
        impl Iterator for #iter_name {
            type Item = #ident;

            fn next(&mut self) -> ::core::option::Option<<Self as Iterator>::Item> {
                self.nth(0)
            }

            fn size_hint(&self) -> (usize, ::core::option::Option<usize>) {
                let t = if self.idx + self.back_idx >= #variant_count { 0 } else { #variant_count - self.idx - self.back_idx };
                (t, Some(t))
            }

            fn nth(&mut self, n: usize) -> ::core::option::Option<<Self as Iterator>::Item> {
                let idx = self.idx + n + 1;
                if idx + self.back_idx > #variant_count {
                    // Past the end, `idx` freezes at the count so repeated
                    // calls cannot overflow it.
                    self.idx = #variant_count;
                    ::core::option::Option::None
                } else {
                    self.idx = idx;
                    self.get(idx - 1)
                }
            }
        }

        #[automatically_derived]
        impl ExactSizeIterator for #iter_name {
            fn len(&self) -> usize {
                self.size_hint().0
            }
        }

        #[automatically_derived]
        impl DoubleEndedIterator for #iter_name {
            fn next_back(&mut self) -> ::core::option::Option<<Self as Iterator>::Item> {
                let back_idx = self.back_idx + 1;

                if self.idx + back_idx > #variant_count {
                    // Past the front, `back_idx` freezes at the count so
                    // repeated calls cannot overflow it.
                    self.back_idx = #variant_count;
                    ::core::option::Option::None
                } else {
                    self.back_idx = back_idx;
                    self.get(#variant_count - self.back_idx)
                }
            }
        }

        #[automatically_derived]
        impl ::core::iter::FusedIterator for #iter_name { }

        #[automatically_derived]
        impl Clone for #iter_name {
            fn clone(&self) -> #iter_name {
                #iter_name {
                    idx: self.idx,
                    back_idx: self.back_idx,
                }
            }
        }
    }
}
