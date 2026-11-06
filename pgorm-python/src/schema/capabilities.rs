// [spec:pgorm:req:bindings.schema]
// [spec:pgorm:req:bindings.schema-views+2]
// [spec:pgorm:req:bindings.schema-schemas]
// [spec:pgorm:req:bindings.schema-domains+2]
pub(crate) fn operations() -> serde_json::Map<String, serde_json::Value> {
    [
        ("schema.data_type", "pgorm_query::ColumnType"),
        ("schema.column", "pgorm_query::ColumnDef"),
        ("schema.create_table", "pgorm_query::TableCreateStatement"),
        ("schema.alter_table", "pgorm_query::{PendingTableAlter,TableAlterStatement}"),
        ("schema.drop_table", "pgorm_query::TableDropStatement"),
        ("schema.rename_table", "pgorm_query::TableRenameStatement"),
        ("schema.rename_column", "pgorm_query::ColumnRenameStatement"),
        ("schema.rename_constraint", "pgorm_query::ConstraintRenameStatement"),
        ("schema.truncate_table", "pgorm_query::TableTruncateStatement"),
        ("schema.comment_on_table", "pgorm_query::Comment::on_table"),
        ("schema.comment_on_column", "pgorm_query::Comment::on_column"),
        ("schema.create_index", "pgorm_query::IndexCreateStatement"),
        ("schema.drop_index", "pgorm_query::IndexDropStatement"),
        ("schema.create_type", "pgorm_query::extension::TypeCreateStatement"),
        ("schema.alter_type", "pgorm_query::extension::{PendingTypeAlter,TypeAlterStatement,CompositeAlterStatement,AttributeRenameStatement}"),
        ("schema.drop_type", "pgorm_query::extension::TypeDropStatement"),
        ("schema.create_domain", "pgorm_query::DomainCreateStatement"),
        ("schema.alter_domain", "pgorm_query::{PendingDomainAlter,DomainAlterStatement}"),
        ("schema.drop_domain", "pgorm_query::DomainDropStatement"),
        ("schema.create_sequence", "pgorm_query::SequenceCreateStatement"),
        ("schema.alter_sequence", "pgorm_query::{PendingSequenceAlter,SequenceAlterStatement}"),
        ("schema.drop_sequence", "pgorm_query::SequenceDropStatement"),
        ("schema.rename_sequence", "pgorm_query::SequenceRenameStatement"),
        ("schema.create_extension", "pgorm_query::extension::ExtensionCreateStatement"),
        ("schema.drop_extension", "pgorm_query::extension::ExtensionDropStatement"),
        ("schema.create_view", "pgorm_query::ViewCreateStatement"),
        ("schema.create_recursive_view", "pgorm_query::RecursiveViewCreateStatement"),
        ("schema.drop_view", "pgorm_query::ViewDropStatement"),
        ("schema.create_materialized_view", "pgorm_query::MaterializedViewCreateStatement"),
        ("schema.refresh_materialized_view", "pgorm_query::MaterializedViewRefreshStatement"),
        ("schema.drop_materialized_view", "pgorm_query::MaterializedViewDropStatement"),
        ("schema.create_table_as", "pgorm_query::TableCreateAsStatement"),
        ("schema.create_schema", "pgorm_query::SchemaCreateStatement"),
        ("schema.drop_schema", "pgorm_query::SchemaDropStatement"),
        ("schema.rename_schema", "pgorm_query::SchemaRenameStatement"),
        ("schema.from_entity", "Schema::{create_table_from_entity,create_enum_from_entity,create_index_from_entity,create_comments_from_entity}"),
        ("schema.from_model", "Schema::{create_table_from_shape,create_enum_from_shape,create_index_from_shape,create_comments_from_shape}"),
    ].into_iter().map(|(name, api)| (name.to_owned(), serde_json::json!({
        "rust_api": format!("pgorm::{api}"), "features": [],
        "registration_required": name == "schema.from_entity"
    }))).collect()
}
