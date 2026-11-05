use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "stock_item")]
pub struct Model {
    #[pgorm(primary_key, identity)]
    pub id: i32,
    #[pgorm(
        default_expr = Func::named(Name::runtime("md5"))
            .arg(
                Expr::expr(Func::named(Name::runtime("random")))
                    .cast_as(Name::runtime("text")),
            ),
        column_type = ColumnType::Text
    )]
    pub code: String,
    #[pgorm(default_value = "it's", column_type = ColumnType::Text)]
    pub name: String,
    #[pgorm(default_value = -1)]
    pub quantity: i32,
    #[pgorm(default_value = 3000000000i64)]
    pub reorder_at: i64,
    #[pgorm(default_value = Decimal::new(150, 2))]
    pub price: Decimal,
    #[pgorm(default_value = true)]
    pub active: bool,
    #[pgorm(default_value = false)]
    pub archived: bool,
    #[pgorm(
        default_expr = pgorm::pgorm_query::Keyword::Null,
        column_type = ColumnType::Text,
        nullable
    )]
    pub note: Option<String>,
    #[pgorm(
        default_expr = Expr::current_date().cast_as(Name::runtime("text")),
        column_type = ColumnType::Text
    )]
    pub stocked_on: String,
    #[pgorm(
        default_expr = Func::named(Name::runtime("cardinality"))
            .arg(
                Expr::val("{}")
                    .cast_as_type(
                        pgorm::pgorm_query::TypeName::new(Name::runtime("text")).array(),
                    ),
            )
    )]
    pub tag_count: i32,
    #[pgorm(default_expr = Expr::val("x").cast_as(Name::runtime("varchar")))]
    pub label: String,
    #[pgorm(generated_stored = Expr::col(Column::Price).mul(Expr::col(Column::Quantity)))]
    pub total: Option<Decimal>,
    #[pgorm(
        generated_virtual = Expr::expr(
                Expr::expr(
                        Func::named(Name::runtime("upper")).arg(Expr::col(Column::Name)),
                    )
                    .concat(Expr::val(" #")),
            )
            .concat(Expr::col(Column::Quantity).cast_as(Name::runtime("text"))),
        column_type = ColumnType::Text,
        nullable
    )]
    pub shown: Option<String>,
    #[pgorm(
        generated_stored = Expr::expr(
                Expr::expr(Expr::col(Column::Quantity).gt(Expr::val(0)))
                    .binary(
                        pgorm::pgorm_query::BinOper::And,
                        Expr::expr(Expr::col(Column::Note).is_null()).not(),
                    ),
            )
            .binary(pgorm::pgorm_query::BinOper::Or, Expr::col(Column::Active))
    )]
    pub in_stock: Option<bool>,
    #[pgorm(
        generated_virtual = Expr::expr(
                Expr::expr(Expr::col(Column::Quantity).add(Expr::val(1)))
                    .mul(Expr::val(2)),
            )
            .modulo(Expr::val(7))
    )]
    pub spare: Option<i32>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
