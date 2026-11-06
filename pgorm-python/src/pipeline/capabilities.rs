pub(crate) fn operations() -> serde_json::Map<String, serde_json::Value> {
    [
        ("pipeline.from", "Pipeline::{from,from_schema}"),
        ("pipeline.source", "IntoSource::into_source / named_runtime"),
        (
            "pipeline.expression",
            "Expr / ExprOps / native aggregate and window functions",
        ),
        ("pipeline.compares_as", "ExprOps::compares_as"),
        ("pipeline.registered_column", "Named::col"),
        ("pipeline.model_column", "col_typed"),
        (
            "pipeline.bind",
            "Binder::bind inside branded Rust stage closures",
        ),
        ("pipeline.filter", "Pipeline::{filter,filter_with}"),
        ("pipeline.derive", "Pipeline::{derive,derive_with}"),
        ("pipeline.select", "Pipeline::{select,select_with}"),
        (
            "pipeline.group",
            "Pipeline::{group,group_with} / Grouped::{aggregate,aggregate_with}",
        ),
        ("pipeline.window", "Pipeline::{window,window_with} / Over"),
        ("pipeline.sort", "Pipeline::{sort,sort_with}"),
        ("pipeline.take", "Pipeline::{take,take_range}"),
        ("pipeline.join", "Pipeline::{join,join_with}"),
        (
            "pipeline.set",
            "Pipeline::{append,intersect,remove,distinct}",
        ),
        ("pipeline.inspect", "Pipeline::into_sql"),
        (
            "pipeline.records",
            "Pipeline::into_sql / ConnectionTrait::query_all / native Record decode",
        ),
        (
            "pipeline.select_sources",
            "Pipeline::select_sources / SelectedSources::{all,one,one_opt}",
        ),
        ("pipeline.source_tuples", "SourceList"),
        (
            "pipeline.select_models",
            "Pipeline::select_described / DescribedSource",
        ),
        (
            "pipeline.into_model",
            "Pipeline::into_sql / ConnectionTrait::query_all / native Record decode",
        ),
        (
            "pipeline.into_tuple",
            "Pipeline::into_sql / ConnectionTrait::query_all / positional decode",
        ),
    ]
    .into_iter()
    .map(|(name, api)| {
        (
            name.to_owned(),
            serde_json::json!({
                "rust_api": format!("pgorm::pipeline::{api}"), "features": [],
                "registration_required": matches!(
                    name,
                    "pipeline.select_sources"
                        | "pipeline.registered_column"
                        | "pipeline.source_tuples"
                )
            }),
        )
    })
    .collect()
}
