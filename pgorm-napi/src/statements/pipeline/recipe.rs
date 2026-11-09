//! A pipeline expression as JavaScript holds it: pgorm-bindings' owned
//! recipe, and the names JavaScript spells its operators and functions by.

pub(super) use pgorm_bindings::pipeline::{Binary, Function, Literal, Recipe, Unary};

pub(super) const BINARY: &[(&str, Binary)] = &[
    ("eq", Binary::Eq),
    ("ne", Binary::Ne),
    ("gt", Binary::Gt),
    ("gte", Binary::Gte),
    ("lt", Binary::Lt),
    ("lte", Binary::Lte),
    ("and", Binary::And),
    ("or", Binary::Or),
    ("coalesce", Binary::Coalesce),
    ("add", Binary::Add),
    ("sub", Binary::Sub),
    ("mul", Binary::Mul),
    ("div", Binary::Div),
    ("rem", Binary::Rem),
];

pub(super) const UNARY: &[(&str, Unary)] = &[
    ("not", Unary::Not),
    ("neg", Unary::Neg),
    ("asc", Unary::Asc),
    ("desc", Unary::Desc),
    ("isNull", Unary::IsNull),
    ("isNotNull", Unary::IsNotNull),
];

pub(super) const FUNCTIONS: &[(&str, Function)] = &[
    ("sum", Function::Sum),
    ("min", Function::Min),
    ("max", Function::Max),
    ("average", Function::Average),
    ("stddev", Function::Stddev),
    ("count", Function::Count),
    ("countDistinct", Function::CountDistinct),
    ("rank", Function::Rank),
    ("rankDense", Function::RankDense),
    ("first", Function::First),
    ("last", Function::Last),
];
