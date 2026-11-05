use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "formula")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub a: Option<i32>,
    pub b: Option<i32>,
    pub c: Option<i32>,
    pub p: Option<bool>,
    pub q: Option<bool>,
    pub r: Option<bool>,
    #[pgorm(column_type = ColumnType::Text, nullable)]
    pub s: Option<String>,
    #[pgorm(column_type = ColumnType::Text, nullable)]
    pub t: Option<String>,
    #[pgorm(
        generated_stored = Expr::expr(Expr::col(Column::P).not())
            .eq(Expr::col(Column::Q))
    )]
    pub not_first: Option<bool>,
    #[pgorm(
        generated_stored = Expr::col(Column::A)
            .sub(Expr::col(Column::B).sub(Expr::col(Column::C)))
    )]
    pub sub_right: Option<i32>,
    #[pgorm(
        generated_stored = Expr::expr(Expr::col(Column::A).sub(Expr::col(Column::B)))
            .sub(Expr::col(Column::C))
    )]
    pub sub_left: Option<i32>,
    #[pgorm(
        generated_stored = Expr::col(Column::A)
            .div(Expr::col(Column::B).mul(Expr::col(Column::C)))
    )]
    pub div_product: Option<i32>,
    #[pgorm(
        generated_stored = Expr::col(Column::A)
            .modulo(Expr::col(Column::B).add(Expr::col(Column::C)))
    )]
    pub mod_sum: Option<i32>,
    #[pgorm(
        generated_stored = Expr::col(Column::S)
            .concat(Expr::col(Column::T).concat(Expr::col(Column::S))),
        column_type = ColumnType::Text,
        nullable
    )]
    pub concat_right: Option<String>,
    #[pgorm(
        generated_stored = Expr::expr(
                Expr::col(Column::P)
                    .binary(pgorm::pgorm_query::BinOper::Or, Expr::col(Column::Q)),
            )
            .binary(pgorm::pgorm_query::BinOper::And, Expr::col(Column::R))
    )]
    pub or_then_and: Option<bool>,
    #[pgorm(
        generated_stored = Expr::col(Column::P)
            .binary(
                pgorm::pgorm_query::BinOper::And,
                Expr::col(Column::Q)
                    .binary(pgorm::pgorm_query::BinOper::Or, Expr::col(Column::R)),
            )
    )]
    pub and_then_or: Option<bool>,
    #[pgorm(
        generated_stored = Expr::expr(
                Expr::col(Column::P)
                    .binary(pgorm::pgorm_query::BinOper::And, Expr::col(Column::Q)),
            )
            .not()
    )]
    pub not_and: Option<bool>,
    #[pgorm(generated_stored = Expr::expr(Expr::col(Column::P).not()).not())]
    pub not_not: Option<bool>,
    #[pgorm(generated_stored = Expr::expr(Expr::col(Column::P).is_null()).not())]
    pub not_is_null: Option<bool>,
    #[pgorm(
        generated_stored = Expr::expr(Expr::col(Column::A).is_null())
            .eq(Expr::col(Column::P))
    )]
    pub is_null_compared: Option<bool>,
    #[pgorm(
        generated_stored = Expr::expr(Expr::col(Column::A).add(Expr::col(Column::B)))
            .is_null()
    )]
    pub sum_is_null: Option<bool>,
    #[pgorm(
        generated_stored = Expr::expr(Expr::col(Column::A).add(Expr::col(Column::B)))
            .cast_as(Name::runtime("text")),
        column_type = ColumnType::Text,
        nullable
    )]
    pub sum_cast: Option<String>,
    #[pgorm(
        generated_stored = Expr::expr(Expr::col(Column::A).eq(Expr::col(Column::B)))
            .eq(Expr::col(Column::P))
    )]
    pub compare_left: Option<bool>,
    #[pgorm(
        generated_stored = Expr::col(Column::P)
            .eq(Expr::col(Column::Q).eq(Expr::col(Column::R)))
    )]
    pub compare_right: Option<bool>,
    #[pgorm(
        generated_stored = Expr::expr(Expr::col(Column::A).lt(Expr::col(Column::B)))
            .eq(Expr::col(Column::B).lt(Expr::col(Column::C)))
    )]
    pub compare_both: Option<bool>,
    #[pgorm(
        generated_stored = Expr::expr(Expr::col(Column::A).concat(Expr::col(Column::S)))
            .eq(Expr::col(Column::T))
    )]
    pub concat_compared: Option<bool>,
    #[pgorm(generated_stored = Expr::val(-1).mul(Expr::col(Column::A)))]
    pub negative_left: Option<i32>,
    #[pgorm(generated_stored = Expr::col(Column::A).sub(Expr::val(-1)))]
    pub negative_right: Option<i32>,
    #[pgorm(
        generated_virtual = Func::named(Name::runtime("coalesce"))
            .arg(Expr::col(Column::A))
            .arg(Expr::col(Column::B))
            .arg(Expr::val(0))
    )]
    pub coalesced: Option<i32>,
    #[pgorm(
        generated_stored = Expr::expr(
                Func::named(Name::runtime("greatest"))
                    .arg(Expr::col(Column::A))
                    .arg(Expr::col(Column::B)),
            )
            .sub(
                Func::named(Name::runtime("least"))
                    .arg(Expr::col(Column::B))
                    .arg(Expr::col(Column::C)),
            )
    )]
    pub spread: Option<i32>,
    #[pgorm(
        generated_virtual = Func::named(Name::runtime("nullif"))
            .arg(Expr::col(Column::A))
            .arg(Expr::val(0))
    )]
    pub nulled: Option<i32>,
    #[pgorm(
        generated_stored = Expr::case(
                Expr::col(Column::A).gt(Expr::col(Column::B)),
                Expr::col(Column::A),
            )
            .case(Expr::col(Column::A).lt(Expr::col(Column::B)), Expr::col(Column::B))
            .finally(Expr::col(Column::C))
    )]
    pub ranked: Option<i32>,
    #[pgorm(
        generated_stored = Expr::case(
            pgorm::pgorm_query::SimpleExpr::from(Expr::col(Column::P)),
            Expr::col(Column::Q),
        )
    )]
    pub tested: Option<bool>,
    #[pgorm(
        generated_virtual = Expr::expr(
                Expr::case(Expr::col(Column::P).is_null(), Expr::val(0))
                    .finally(Expr::col(Column::A)),
            )
            .add(Expr::val(1))
    )]
    pub case_sum: Option<i32>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
