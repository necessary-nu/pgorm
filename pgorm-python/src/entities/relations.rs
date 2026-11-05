//! What `describe()` reports of a registered entity's relations: the parts of
//! a compiled declaration no Python call names. The columns, the `PERIOD`
//! pair, the condition type and an attached `on_condition` decide what a join
//! by the relation reads; enforcement and deferrability decide what the
//! server holds the foreign key's rows to, and leave the join as it is.

use pgorm::pgorm_query::{ConditionType, Deferrability, Enforcement, FromItem};
use pgorm::{EntityTrait, Iterable, RelationDef, RelationTrait, RelationType};
use serde_json::{Value as Json, json};

// [spec:pgorm:req:bindings.entities+6]
// [spec:pgorm:req:bindings.relation-conditions]
/// Every relation the entity's `Relation` enum declares, in declaration order.
pub(super) fn describe<E: EntityTrait>() -> Vec<Json> {
    E::Relation::iter()
        .map(|relation| relation_json(&format!("{relation:?}"), &relation.def()))
        .collect()
}

/// `on_condition` is reported as present or absent only: it is a Rust
/// closure over the two tables' aliases, which has no rendering of its own
/// until a join supplies them.
fn relation_json(name: &str, def: &RelationDef) -> Json {
    json!({
        "name": name,
        "type": match def.rel_type {
            RelationType::HasOne => "has_one",
            RelationType::HasMany => "has_many",
        },
        "from": table(&def.from_tbl),
        "to": table(&def.to_tbl),
        "columns": def
            .columns
            .iter()
            .map(|(from, to)| [from.to_string(), to.to_string()])
            .collect::<Vec<_>>(),
        "period": def
            .columns
            .period()
            .map(|(from, to)| [from.to_string(), to.to_string()]),
        "condition_type": match def.condition_type {
            ConditionType::All => "all",
            ConditionType::Any => "any",
        },
        "on_condition": def.on_condition.is_some(),
        "enforcement": def.enforcement.map(|enforcement| match enforcement {
            Enforcement::Enforced => "enforced",
            Enforcement::NotEnforced => "not_enforced",
        }),
        "deferrability": def.deferrability.map(|deferrability| match deferrability {
            Deferrability::NotDeferrable => "not_deferrable",
            Deferrability::DeferrableInitiallyImmediate => "deferrable_initially_immediate",
            Deferrability::DeferrableInitiallyDeferred => "deferrable_initially_deferred",
        }),
    })
}

/// A relation's table as `{"schema", "table"}`; a relation joins entities,
/// whose FROM item is always a named table.
fn table(item: &FromItem) -> Json {
    match item {
        FromItem::Table(table) => json!({
            "schema": table.name.schema().map(|schema| schema.to_string()),
            "table": table.name.table().to_string(),
        }),
        other => json!({"schema": null, "table": other.qualifier().to_string()}),
    }
}

#[cfg(test)]
mod tests {
    use super::relation_json;
    use pgorm::pgorm_query::{Condition, ConditionType, FromItem, IntoName, TableName};
    use pgorm::{ColumnPairs, RelationDef, RelationType, alias};

    fn relation() -> RelationDef {
        RelationDef {
            rel_type: RelationType::HasOne,
            from_tbl: FromItem::from(TableName::Table(alias("note").into_name())),
            to_tbl: FromItem::from(TableName::Table(alias("account").into_name())),
            columns: ColumnPairs::new(alias("account_id"), alias("id")),
            is_owner: true,
            on_delete: None,
            on_update: None,
            on_condition: None,
            fk_name: None,
            enforcement: None,
            deferrability: None,
            condition_type: ConditionType::All,
        }
    }

    // [spec:pgorm:req:bindings.entities+6/test]
    // [spec:pgorm:req:bindings.relation-conditions/test]
    #[test]
    fn join_condition_shape_is_reported() {
        let plain = relation_json("Account", &relation());
        assert_eq!(plain["condition_type"], "all");
        assert_eq!(plain["on_condition"], false);
        let mut either = relation();
        either.condition_type = ConditionType::Any;
        either.on_condition = Some(Box::new(|_, _| Condition::all()));
        let either = relation_json("Account", &either);
        assert_eq!(either["condition_type"], "any");
        assert_eq!(either["on_condition"], true);
        assert_eq!(either["columns"], plain["columns"]);
    }
}
