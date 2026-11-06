//! The `prql!` half: compile PRQL text, hold the result to the grammar, and
//! check the placeholders against the arguments.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Error, Expr, LitStr, Token};

/// The macro's input: one PRQL string literal, then one argument per `$N`.
pub(crate) struct PrqlInput {
    literal: LitStr,
    args: Vec<Expr>,
}

impl Parse for PrqlInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let literal = input.parse()?;
        let mut args = Vec::new();
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            args.push(input.parse()?);
        }

        Ok(Self { literal, args })
    }
}

/// Compile, validate, and expand — or refuse with a spanned error.
///
/// The expansion is a plain tuple expression: the emitted SQL as a string
/// literal (spanned on the PRQL literal it came from) and a
/// `::pgorm::Values` holding each argument converted through
/// `Into<::pgorm::Value>`, in placeholder order. No runtime helper, no new
/// types — a failed conversion or a missing `pgorm` dependency is an
/// ordinary type error at the call site.
// [spec:pgorm:def:macros.prql+1]
pub(crate) fn expand(input: &PrqlInput) -> Result<TokenStream, Error> {
    let compiled = pgorm_sql_check::compile_prql(&input.literal.value())
        .map_err(|rejection| Error::new(input.literal.span(), rejection))?;
    check_arity(input, compiled.placeholders())?;

    let literal = LitStr::new(compiled.sql(), input.literal.span());
    let values = input
        .args
        .iter()
        .map(|arg| quote!(::std::convert::Into::<::pgorm::Value>::into(#arg)));

    Ok(quote! {
        (#literal, ::pgorm::Values(::std::vec![#(#values),*]))
    })
}

/// One argument per distinct placeholder, in placeholder order.
///
/// Too few arguments is an error on the literal naming every unbound `$N`;
/// too many is an error on each surplus argument naming the `$N` the query
/// does not have. Reuse is not arity: `$1` twice is one argument.
// [spec:pgorm:req:macros.prql.reject]    arity errors name the placeholder
fn check_arity(input: &PrqlInput, expected: usize) -> Result<(), Error> {
    let given = input.args.len();
    if given < expected {
        let unbound = pgorm_sql_check::ArityRejection::Unbound { expected, given };
        return Err(Error::new(input.literal.span(), unbound));
    }

    let mut surplus = input
        .args
        .iter()
        .enumerate()
        .skip(expected)
        .map(|(index, argument)| {
            let surplus = pgorm_sql_check::ArityRejection::Surplus {
                expected,
                number: index + 1,
            };
            Error::new_spanned(argument, surplus)
        });
    if let Some(mut error) = surplus.next() {
        for another in surplus {
            error.combine(another);
        }
        return Err(error);
    }

    Ok(())
}
