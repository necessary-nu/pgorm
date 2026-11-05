// [spec:pgorm:req:bindings.transactions]
pub(crate) fn operations() -> serde_json::Map<String, serde_json::Value> {
    [
        ("transaction", "DatabaseConnection::transaction_with"),
        (
            "transaction.savepoint",
            "TransactionTrait::transaction for DatabaseTransaction",
        ),
        (
            "transaction.commit",
            "TransactionTrait::transaction, the closure returning Ok",
        ),
        (
            "transaction.rollback",
            "TransactionTrait::transaction, the closure returning Err",
        ),
        (
            "transaction.execute",
            "ConnectionTrait::execute for DatabaseTransaction",
        ),
        (
            "transaction.query",
            "ConnectionTrait::query_all for DatabaseTransaction",
        ),
        (
            "transaction.one",
            "ConnectionTrait::query_all for DatabaseTransaction",
        ),
        (
            "transaction.optional",
            "ConnectionTrait::query_all for DatabaseTransaction",
        ),
        (
            "transaction.registered",
            "Select / ActiveModel / SelectGraph / SelectedSources over DatabaseTransaction",
        ),
    ]
    .into_iter()
    .map(|(name, api)| {
        (
            name.to_owned(),
            serde_json::json!({
                "rust_api": format!("pgorm::{api}"), "features": []
            }),
        )
    })
    .collect()
}
