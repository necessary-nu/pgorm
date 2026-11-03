//! Every attribute value that is Rust code, written bare, through each derive
//! that takes one: taken as written, with no implicit prefix.

#![allow(dead_code)]

use pgorm::entity::prelude::*;
use pgorm::{PartialModelTrait, QueryTrait};

/// A generic type, which parses as a type and not as an expression.
type Same<T> = T;

mod parent {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "parent")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod child {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "child")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(variant = ParentKey)]
        pub parent_id: i32,
        #[pgorm(default_value = -1)]
        pub score: i32,
        #[pgorm(default_value = "untitled")]
        pub title: String,
        #[pgorm(default_expr = Expr::val("x"))]
        pub label: String,
        #[pgorm(generated_stored = Expr::col(Column::Score).mul(2))]
        pub doubled: i32,
        #[pgorm(column_type = ColumnType::Text)]
        pub note: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::parent::Entity,
            from = Column::ParentKey,
            to = super::parent::Column::Id,
            on_update = ForeignKeyAction::Cascade,
            on_delete = ForeignKeyAction::SetNull,
            enforcement = Enforcement::NotEnforced,
            deferrability = Deferrability::DeferrableInitiallyImmediate,
            condition_type = ConditionType::Any,
            fk_name = "child_parent",
            on_condition = |_left, right| Expr::col((right, super::parent::Column::Id)).gt(0)
        )]
        Parent,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
#[pgorm(rs_type = Same<String>, db_type = ColumnType::String(StringLen::N(1)))]
pub enum Size {
    #[pgorm(string_value = "S")]
    Small,
    #[pgorm(string_value = "L")]
    Large,
}

#[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
#[pgorm(enum_name = "tea")]
pub enum Tea {
    #[pgorm(string_value = "green")]
    Green,
}

#[derive(Clone, Debug, PartialEq, ValueType)]
#[pgorm(column_type = ColumnType::Text, array_type = ArrayType::String)]
pub struct Note(String);

#[derive(Debug, PartialModel)]
#[pgorm(entity = child::Entity)]
pub struct ChildView {
    id: i32,
    #[pgorm(from_col = child::Column::Note)]
    text: String,
    #[pgorm(from_expr = Expr::col(child::Column::Score).add(1))]
    next: i32,
}

// [spec:pgorm:req:macros.derive.code-values+3/test]    a relation's target, column pair,
// actions, key check, condition type and closure condition, all bare
#[test]
fn relation_keys_are_taken_as_written() {
    use pgorm::RelationTrait;

    let def = child::Relation::Parent.def();
    assert!(matches!(def.on_update, Some(ForeignKeyAction::Cascade)));
    assert!(matches!(def.on_delete, Some(ForeignKeyAction::SetNull)));
    assert_eq!(def.enforcement, Some(Enforcement::NotEnforced));
    assert_eq!(
        def.deferrability,
        Some(Deferrability::DeferrableInitiallyImmediate)
    );
    assert_eq!(def.condition_type, ConditionType::Any);
    assert_eq!(def.fk_name.as_deref(), Some("child_parent"));
    assert!(def.on_condition.is_some());
}

// [spec:pgorm:req:macros.derive.code-values+3/test]    `variant` names the field's `Column`
// variant, and the defaults and generation are expressions, a negative literal included
#[test]
fn field_keys_are_taken_as_written() {
    // With no `column_name`, the SQL name is the variant's, as it is for a
    // variant the field's name derives.
    assert_eq!(child::Column::ParentKey.as_str(), "parent_key");
    assert_eq!(
        child::Column::Score.def(),
        ColumnType::Integer.def().default_value(-1)
    );
    assert_eq!(
        child::Column::Title.def(),
        ColumnType::string(None).def().default_value("untitled")
    );
    assert_eq!(
        child::Column::Label.def(),
        ColumnType::string(None).def().default(Expr::val("x"))
    );
    assert_eq!(
        child::Column::Doubled.def(),
        ColumnType::Integer.def().generated(
            Expr::col(child::Column::Score).mul(2),
            pgorm::pgorm_query::GeneratedKind::Stored
        )
    );
    assert_eq!(child::Column::Note.def(), ColumnType::Text.def());
}

// [spec:pgorm:req:macros.derive.code-values+3/test]    `rs_type` is a type, generics
// included; `db_type` an expression, omitted for a PostgreSQL enum; `array_type` an expression
#[test]
fn type_keys_are_taken_as_written() {
    assert_eq!(Size::Large.to_value(), "L".to_owned());
    assert_eq!(
        Size::db_type().get_column_type(),
        &ColumnType::String(StringLen::N(1))
    );
    assert!(matches!(
        Tea::db_type().get_column_type(),
        ColumnType::Enum { .. }
    ));
    assert_eq!(
        <Note as pgorm::pgorm_query::ValueType>::array_type(),
        ArrayType::String
    );
}

// [spec:pgorm:req:macros.derive.code-values+3/test]    `PartialModel`'s entity is a type,
// `from_col` a column path and `from_expr` an expression
#[test]
fn projection_keys_are_taken_as_written() {
    let sql = ChildView::select_cols(child::Entity::find().select_only())
        .build()
        .0;
    assert_eq!(
        sql,
        r#"SELECT "child"."id", "child"."note" AS "text", "score" + $1 AS "next" FROM "child""#
    );
}
