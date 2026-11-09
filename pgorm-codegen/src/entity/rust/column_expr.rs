use heck::ToUpperCamelCase;
use proc_macro2::{Literal, TokenStream};
use quote::{ToTokens, format_ident, quote};

use crate::{ColumnExpr, CurrentKeyword, Operator, util::escape_rust_keyword};

impl Operator {
    /// The builder method on `Expr` that writes this operator, or the
    /// `BinOper` variant `ExprTrait::binary` takes for the two that have none.
    fn method(self) -> Result<&'static str, &'static str> {
        match self {
            Self::Add => Ok("add"),
            Self::Sub => Ok("sub"),
            Self::Mul => Ok("mul"),
            Self::Div => Ok("div"),
            Self::Mod => Ok("modulo"),
            Self::Eq => Ok("eq"),
            Self::Ne => Ok("ne"),
            Self::Lt => Ok("lt"),
            Self::Gt => Ok("gt"),
            Self::Le => Ok("lte"),
            Self::Ge => Ok("gte"),
            Self::Concat => Ok("concat"),
            Self::And => Err("And"),
            Self::Or => Err("Or"),
        }
    }
}

impl ColumnExpr {
    /// The Rust that builds [`lower`](Self::lower)'s `SimpleExpr`, inside an
    /// entity whose `Column` enum names the table's columns.
    // [spec:pgorm:sem:codegen.entity.expressions+3]
    pub(crate) fn to_rust(&self) -> TokenStream {
        match self {
            Self::Column(name) => {
                let column = format_ident!("{}", escape_rust_keyword(name.to_upper_camel_case()));
                quote! { Expr::col(Column::#column) }
            }
            Self::Int(_) | Self::BigInt(_) | Self::Decimal(..) | Self::Text(_) | Self::Bool(_) => {
                let value = self.to_value().expect("a literal is a value");
                quote! { Expr::val(#value) }
            }
            Self::Null => quote! { pgorm::pgorm_query::Keyword::Null },
            Self::Keyword(keyword) => match keyword {
                CurrentKeyword::Date => quote! { Expr::current_date() },
                CurrentKeyword::Time => quote! { Expr::current_time() },
                CurrentKeyword::Timestamp => quote! { Expr::current_timestamp() },
            },
            Self::Call(name, args) => {
                let args = args.iter().map(Self::to_rust);
                quote! { Func::named(Name::runtime(#name)) #( .arg(#args) )* }
            }
            Self::Cast(operand, cast) => {
                let operand = operand.to_expr();
                let name = &cast.name;
                match (&cast.schema, cast.array) {
                    (None, false) => quote! { #operand.cast_as(Name::runtime(#name)) },
                    (schema, array) => {
                        let schema = schema
                            .iter()
                            .map(|schema| quote! { .schema(Name::runtime(#schema)) });
                        let array = array.then(|| quote! { .array() });
                        quote! {
                            #operand.cast_as_type(
                                pgorm::pgorm_query::TypeName::new(Name::runtime(#name))
                                    #( #schema )* #array
                            )
                        }
                    }
                }
            }
            Self::Binary(left, op, right) => {
                let (left, right) = (left.to_expr(), right.to_rust());
                match op.method() {
                    Ok(method) => {
                        let method = format_ident!("{}", method);
                        quote! { #left.#method(#right) }
                    }
                    Err(variant) => {
                        let variant = format_ident!("{}", variant);
                        quote! {
                            #left.binary(pgorm::pgorm_query::BinOper::#variant, #right)
                        }
                    }
                }
            }
            Self::Not(operand) => {
                let operand = operand.to_expr();
                quote! { #operand.not() }
            }
            Self::IsNull(operand, negated) => {
                let operand = operand.to_expr();
                let method = if *negated {
                    format_ident!("is_not_null")
                } else {
                    format_ident!("is_null")
                };
                quote! { #operand.#method() }
            }
            Self::Case(case) => {
                let (condition, result) = (case.first.0.to_condition(), case.first.1.to_rust());
                let rest = case.rest.iter().map(|(condition, result)| {
                    let (condition, result) = (condition.to_condition(), result.to_rust());
                    quote! { .case(#condition, #result) }
                });
                let otherwise = case.otherwise.as_ref().map(|otherwise| {
                    let otherwise = otherwise.to_rust();
                    quote! { .finally(#otherwise) }
                });
                quote! { Expr::case(#condition, #result) #( #rest )* #otherwise }
            }
        }
    }

    /// The Rust of the value a literal is, which `Expr::val` takes and the
    /// derive's `default_value` is written as: an `integer` bare, a `bigint`
    /// `i64`-suffixed, since a bare literal of its size is no `i32`, a
    /// `numeric` as `Decimal::new(m, s)`, a string and a boolean as Rust's
    /// own. `None` for an expression that is not a literal.
    // [spec:pgorm:sem:codegen.entity.expressions+3]
    pub(crate) fn to_value(&self) -> Option<TokenStream> {
        Some(match self {
            Self::Int(int) => Literal::i32_unsuffixed(*int).into_token_stream(),
            Self::BigInt(big) => Literal::i64_suffixed(*big).into_token_stream(),
            Self::Decimal(mantissa, scale) => {
                let mantissa = Literal::i64_unsuffixed(*mantissa);
                let scale = Literal::u32_unsuffixed(*scale);
                quote! { Decimal::new(#mantissa, #scale) }
            }
            Self::Text(text) => Literal::string(text).into_token_stream(),
            Self::Bool(value) => quote! { #value },
            _ => return None,
        })
    }

    /// [`to_rust`](Self::to_rust)'s Rust as a CASE arm's condition, which
    /// `Expr::case` takes as an `IntoCondition`: the operators, casts and
    /// tests build a `SimpleExpr`, which is one, and anything else is made
    /// one through `SimpleExpr::from`.
    fn to_condition(&self) -> TokenStream {
        match self {
            Self::Binary(..) | Self::Not(_) | Self::IsNull(..) | Self::Cast(..) => self.to_rust(),
            _ => {
                let expr = self.to_expr();
                quote! { pgorm::pgorm_query::SimpleExpr::from(#expr) }
            }
        }
    }

    /// [`to_rust`](Self::to_rust)'s Rust as an `Expr`, which the operators
    /// and casts are methods of: a column, a literal and a keyword are built
    /// as one already, and anything else is wrapped in `Expr::expr`.
    fn to_expr(&self) -> TokenStream {
        let rust = self.to_rust();
        match self {
            Self::Column(_)
            | Self::Int(_)
            | Self::BigInt(_)
            | Self::Decimal(..)
            | Self::Text(_)
            | Self::Bool(_)
            | Self::Keyword(_) => rust,
            _ => quote! { Expr::expr(#rust) },
        }
    }
}
