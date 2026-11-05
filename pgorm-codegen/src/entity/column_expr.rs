//! A column's expression — its `DEFAULT`, or the expression a generated column
//! is computed from — in the closed subset codegen reads: columns, literals,
//! `CURRENT_*` keywords, calls by name, casts, the arithmetic, comparison,
//! concatenation and boolean operators, `NOT`, `IS [NOT] NULL` and the
//! searched `CASE`.
//!
//! The DDL bridge reads PostgreSQL's parse tree into it and lowers it to the
//! `SimpleExpr` the bridged statement carries; the transform reads a
//! statement's `SimpleExpr` back into it; and the writers print it as the Rust
//! that builds the same `SimpleExpr`, so an entity generated from a schema
//! generates that schema's expressions back. Anything outside the subset is
//! named, never approximated.

use heck::ToUpperCamelCase;
use pgorm_query::{
    BinOper, CaseStatement, ColumnRef, Expr, Func, Function, Keyword, Name, SimpleExpr, TypeName,
    UnOper, Value,
};
use proc_macro2::{Literal, TokenStream};
use quote::{ToTokens, format_ident, quote};
use rust_decimal::Decimal;

use crate::util::escape_rust_keyword;

/// One expression of the subset.
// [spec:pgorm:sem:codegen.entity.expressions+3]
#[derive(Clone, Debug, PartialEq)]
pub enum ColumnExpr {
    /// A column of the same table, by name.
    Column(String),
    /// An `integer` literal.
    Int(i32),
    /// A `bigint` literal: an integer past `integer`'s range.
    BigInt(i64),
    /// A `numeric` literal, as its mantissa and scale, so `1.50` stays `1.50`.
    Decimal(i64, u32),
    /// A string literal.
    Text(String),
    /// `TRUE` or `FALSE`.
    Bool(bool),
    /// `NULL`.
    Null,
    /// `CURRENT_DATE`, `CURRENT_TIME` or `CURRENT_TIMESTAMP`.
    Keyword(CurrentKeyword),
    /// A call of a function by its unqualified name.
    Call(String, Vec<ColumnExpr>),
    /// A cast to a type named by its name, schema and whether it is an array.
    Cast(Box<ColumnExpr>, CastType),
    /// An operator of [`Operator`]'s, between two expressions.
    Binary(Box<ColumnExpr>, Operator, Box<ColumnExpr>),
    /// `NOT`.
    Not(Box<ColumnExpr>),
    /// `IS NULL`, or `IS NOT NULL` when the flag is set.
    IsNull(Box<ColumnExpr>, bool),
    /// The searched `CASE`; the simple form, comparing one operand with each
    /// arm's value, is outside the subset.
    Case(Box<CaseExpr>),
}

/// A searched `CASE`: its `WHEN <condition> THEN <result>` arms in order,
/// the first held apart because the grammar requires one, and its `ELSE`
/// result.
#[derive(Clone, Debug, PartialEq)]
pub struct CaseExpr {
    pub first: (ColumnExpr, ColumnExpr),
    pub rest: Vec<(ColumnExpr, ColumnExpr)>,
    pub otherwise: Option<ColumnExpr>,
}

impl CaseExpr {
    fn arms(&self) -> impl Iterator<Item = &(ColumnExpr, ColumnExpr)> {
        std::iter::once(&self.first).chain(&self.rest)
    }
}

/// The type a cast names.
#[derive(Clone, Debug, PartialEq)]
pub struct CastType {
    pub schema: Option<String>,
    pub name: String,
    pub array: bool,
}

/// The keywords of the subset: the three `CURRENT_*` values, which are all
/// the writers spell, where pgorm-query's [`Keyword`] is a vocabulary that
/// grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurrentKeyword {
    Date,
    Time,
    Timestamp,
}

impl CurrentKeyword {
    fn keyword(self) -> Keyword {
        match self {
            Self::Date => Keyword::CurrentDate,
            Self::Time => Keyword::CurrentTime,
            Self::Timestamp => Keyword::CurrentTimestamp,
        }
    }
}

/// The binary operators of the subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Concat,
    And,
    Or,
}

impl Operator {
    /// The operator PostgreSQL's grammar spells `op`, if it is one of these.
    pub(crate) fn from_sql(op: &str) -> Option<Self> {
        Some(match op {
            "+" => Self::Add,
            "-" => Self::Sub,
            "*" => Self::Mul,
            "/" => Self::Div,
            "%" => Self::Mod,
            "=" => Self::Eq,
            "<>" | "!=" => Self::Ne,
            "<" => Self::Lt,
            ">" => Self::Gt,
            "<=" => Self::Le,
            ">=" => Self::Ge,
            "||" => Self::Concat,
            _ => return None,
        })
    }

    fn bin_oper(self) -> BinOper {
        match self {
            Self::Add => BinOper::Add,
            Self::Sub => BinOper::Sub,
            Self::Mul => BinOper::Mul,
            Self::Div => BinOper::Div,
            Self::Mod => BinOper::Mod,
            Self::Eq => BinOper::Equal,
            Self::Ne => BinOper::NotEqual,
            Self::Lt => BinOper::SmallerThan,
            Self::Gt => BinOper::GreaterThan,
            Self::Le => BinOper::SmallerThanOrEqual,
            Self::Ge => BinOper::GreaterThanOrEqual,
            Self::Concat => BinOper::Concatenate,
            Self::And => BinOper::And,
            Self::Or => BinOper::Or,
        }
    }

    fn from_bin_oper(oper: BinOper) -> Option<Self> {
        [
            Self::Add,
            Self::Sub,
            Self::Mul,
            Self::Div,
            Self::Mod,
            Self::Eq,
            Self::Ne,
            Self::Lt,
            Self::Gt,
            Self::Le,
            Self::Ge,
            Self::Concat,
            Self::And,
            Self::Or,
        ]
        .into_iter()
        .find(|op| op.bin_oper() == oper)
    }

    /// The builder method on `Expr` that writes this operator, or the
    /// `BinOper` variant `Expr::binary` takes for the two that have none.
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
    /// The numeric literal `text` spells, exactly: an integer within
    /// `integer` or `bigint`, or a decimal of at most 18 digits; an exponent
    /// or a wider literal is refused, as no value here holds it exactly.
    pub(crate) fn numeric(text: &str) -> Option<Self> {
        if let Ok(int) = text.parse::<i32>() {
            return Some(Self::Int(int));
        }
        if let Ok(big) = text.parse::<i64>() {
            return Some(Self::BigInt(big));
        }
        let (whole, fraction) = text.split_once('.')?;
        let digits = format!("{whole}{fraction}");
        if fraction.is_empty()
            || !digits
                .trim_start_matches('-')
                .bytes()
                .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let mantissa = digits.parse::<i64>().ok()?;
        Some(Self::Decimal(mantissa, u32::try_from(fraction.len()).ok()?))
    }

    /// The `SimpleExpr` this builds: the value the bridged statement carries,
    /// and the one the Rust [`to_rust`](Self::to_rust) prints builds.
    // [spec:pgorm:sem:codegen.entity.expressions+3]
    pub(crate) fn lower(&self) -> SimpleExpr {
        match self {
            Self::Column(name) => Expr::col(Name::runtime(name.as_str())).into(),
            Self::Int(int) => Expr::val(*int).into(),
            Self::BigInt(big) => Expr::val(*big).into(),
            Self::Decimal(mantissa, scale) => Expr::val(Decimal::new(*mantissa, *scale)).into(),
            Self::Text(text) => Expr::val(text.as_str()).into(),
            Self::Bool(value) => Expr::val(*value).into(),
            Self::Null => SimpleExpr::Keyword(Keyword::Null),
            Self::Keyword(keyword) => SimpleExpr::Keyword(keyword.keyword()),
            Self::Call(name, args) => args
                .iter()
                .fold(Func::named(Name::runtime(name.as_str())), |call, arg| {
                    call.arg(arg.lower())
                })
                .into(),
            Self::Cast(operand, cast) => Expr::expr(operand.lower()).cast_as_type(cast.type_name()),
            Self::Binary(left, op, right) => {
                Expr::expr(left.lower()).binary(op.bin_oper(), right.lower())
            }
            Self::Not(operand) => Expr::expr(operand.lower()).not(),
            Self::IsNull(operand, false) => Expr::expr(operand.lower()).is_null(),
            Self::IsNull(operand, true) => Expr::expr(operand.lower()).is_not_null(),
            Self::Case(case) => {
                let (condition, result) = &case.first;
                let arms = case.rest.iter().fold(
                    Expr::case(condition.lower(), result.lower()),
                    |arms: CaseStatement, (condition, result)| {
                        arms.case(condition.lower(), result.lower())
                    },
                );
                match &case.otherwise {
                    Some(otherwise) => arms.finally(otherwise.lower()).into(),
                    None => arms.into(),
                }
            }
        }
    }

    /// Read a statement's expression back, or name the part of it outside
    /// the subset.
    // [spec:pgorm:sem:codegen.entity.expressions+3]
    pub(crate) fn read(expr: &SimpleExpr) -> Result<Self, String> {
        Ok(match expr {
            SimpleExpr::Column(ColumnRef::Column(name)) => Self::Column(name.to_string()),
            SimpleExpr::Column(_) => return Err("a qualified column".to_owned()),
            SimpleExpr::Value(value) | SimpleExpr::Constant(value) => Self::value(value)?,
            SimpleExpr::Keyword(Keyword::Null) => Self::Null,
            SimpleExpr::Keyword(Keyword::Value) => {
                return Err("`VALUE`, which only a domain's CHECK reads".to_owned());
            }
            SimpleExpr::Keyword(Keyword::CurrentDate) => Self::Keyword(CurrentKeyword::Date),
            SimpleExpr::Keyword(Keyword::CurrentTime) => Self::Keyword(CurrentKeyword::Time),
            SimpleExpr::Keyword(Keyword::CurrentTimestamp) => {
                Self::Keyword(CurrentKeyword::Timestamp)
            }
            SimpleExpr::FunctionCall(call) => match call.get_func() {
                Function::Named(_) if !call.is_plain() => {
                    return Err("a call carrying DISTINCT or an aggregate clause".to_owned());
                }
                Function::Named(name) => Self::Call(
                    name.to_string(),
                    call.get_args()
                        .iter()
                        .map(Self::read)
                        .collect::<Result<_, _>>()?,
                ),
                _ => return Err("a call of a function other than by name".to_owned()),
            },
            SimpleExpr::AsEnum(type_name, operand) if !type_name.is_verbatim() => Self::Cast(
                Box::new(Self::read(operand)?),
                CastType {
                    schema: type_name.schema.as_ref().map(|schema| schema.to_string()),
                    name: type_name.name.to_string(),
                    array: type_name.array,
                },
            ),
            SimpleExpr::Binary(left, BinOper::Is, right)
            | SimpleExpr::Binary(left, BinOper::IsNot, right)
                if matches!(**right, SimpleExpr::Keyword(Keyword::Null)) =>
            {
                let negated = matches!(expr, SimpleExpr::Binary(_, BinOper::IsNot, _));
                Self::IsNull(Box::new(Self::read(left)?), negated)
            }
            SimpleExpr::Binary(left, oper, right) => match Operator::from_bin_oper(*oper) {
                Some(op) => Self::Binary(
                    Box::new(Self::read(left)?),
                    op,
                    Box::new(Self::read(right)?),
                ),
                None => return Err(format!("the operator {oper:?}")),
            },
            SimpleExpr::Unary(UnOper::Not, operand) => Self::Not(Box::new(Self::read(operand)?)),
            SimpleExpr::Case(case) => {
                let mut arms = case
                    .get_arms()
                    .map(|(condition, result)| Ok((Self::read(&condition)?, Self::read(result)?)));
                let first = arms
                    .next()
                    .unwrap_or_else(|| Err("a CASE with no arm".to_owned()))?;
                Self::Case(Box::new(CaseExpr {
                    first,
                    rest: arms.collect::<Result<_, String>>()?,
                    otherwise: case.get_else().map(Self::read).transpose()?,
                }))
            }
            other => return Err(format!("the expression {other:?}")),
        })
    }

    fn value(value: &Value) -> Result<Self, String> {
        Ok(match value {
            Value::Int(Some(int)) => Self::Int(*int),
            Value::BigInt(Some(big)) => Self::BigInt(*big),
            Value::String(Some(text)) => Self::Text(text.as_ref().clone()),
            Value::Bool(Some(value)) => Self::Bool(*value),
            Value::Decimal(Some(decimal)) => {
                let mantissa = i64::try_from(decimal.mantissa())
                    .map_err(|_| "a decimal wider than 18 digits".to_owned())?;
                Self::Decimal(mantissa, decimal.scale())
            }
            Value::Int(None)
            | Value::BigInt(None)
            | Value::String(None)
            | Value::Bool(None)
            | Value::Decimal(None) => Self::Null,
            other => return Err(format!("the value {other:?}")),
        })
    }

    /// Every column the expression reads, by name.
    pub(crate) fn columns(&self) -> Vec<&str> {
        let mut columns = Vec::new();
        self.collect_columns(&mut columns);
        columns
    }

    fn collect_columns<'a>(&'a self, columns: &mut Vec<&'a str>) {
        match self {
            Self::Column(name) => columns.push(name),
            Self::Call(_, args) => args.iter().for_each(|arg| arg.collect_columns(columns)),
            Self::Cast(operand, _) | Self::Not(operand) | Self::IsNull(operand, _) => {
                operand.collect_columns(columns)
            }
            Self::Binary(left, _, right) => {
                left.collect_columns(columns);
                right.collect_columns(columns);
            }
            Self::Case(case) => {
                for (condition, result) in case.arms() {
                    condition.collect_columns(columns);
                    result.collect_columns(columns);
                }
                if let Some(otherwise) = &case.otherwise {
                    otherwise.collect_columns(columns);
                }
            }
            _ => {}
        }
    }

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

impl CastType {
    fn type_name(&self) -> TypeName {
        let mut type_name = TypeName::new(Name::runtime(self.name.as_str()));
        if let Some(schema) = &self.schema {
            type_name = type_name.schema(Name::runtime(schema.as_str()));
        }
        if self.array {
            type_name = type_name.array();
        }
        type_name
    }
}
