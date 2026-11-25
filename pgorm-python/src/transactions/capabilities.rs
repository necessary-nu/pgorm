// [spec:pgorm:req:bindings.transactions+2]
pub(crate) fn operations() -> serde_json::Map<String, serde_json::Value> {
    [
        ("transaction", "DatabaseConnection::transaction_with"),
        (
            "transaction.savepoint",
            "Transact::transaction for DatabaseTransaction",
        ),
        (
            "transaction.commit",
            "Transact::transaction, the closure returning Ok",
        ),
        (
            "transaction.rollback",
            "Transact::transaction, the closure returning Err",
        ),
        (
            "transaction.execute",
            "Connection::execute for DatabaseTransaction",
        ),
        (
            "transaction.query",
            "Connection::query_all for DatabaseTransaction",
        ),
        (
            "transaction.one",
            "Connection::query_all for DatabaseTransaction",
        ),
        (
            "transaction.optional",
            "Connection::query_all for DatabaseTransaction",
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
