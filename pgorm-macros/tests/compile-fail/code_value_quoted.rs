#![allow(dead_code, unused_imports)]

use pgorm::entity::prelude::*;

mod relation_target {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(belongs_to = "super::Parent", from = Column::ParentId, to = Column::Id)]
        Parent,
    }
}

mod relation_action {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::Child, on_delete = "Cascade")]
        Child,
    }
}

mod relation_condition {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::Child, condition_type = "any")]
        Child,
    }
}

#[derive(Debug, ActiveEnum)]
#[pgorm(rs_type = "String")]
pub enum RsType {
    #[pgorm(string_value = "a")]
    A,
}

#[derive(Debug, ActiveEnum)]
#[pgorm(rs_type = i32, db_type = "Integer")]
pub enum DbType {
    #[pgorm(num_value = 1)]
    A,
}

#[derive(ValueType)]
#[pgorm(array_type = "Int")]
pub struct ArrayTyped(i32);

#[derive(PartialModel)]
#[pgorm(entity = "Entity")]
pub struct QuotedEntity {
    id: i32,
}

#[derive(PartialModel)]
pub struct QuotedExpr {
    #[pgorm(from_expr = "Expr::val(1)")]
    one: i32,
}

#[derive(Model)]
pub struct QuotedVariant {
    #[pgorm(variant = "Id")]
    id: i32,
}

mod quoted_default {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "quoted")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(default_expr = "Func::uuidv7()")]
        pub key: Uuid,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

fn main() {}
