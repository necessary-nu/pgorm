use serde_json::{Map, Value, json};

pub(crate) fn operations() -> Map<String, Value> {
    [
        ("table", "pgorm_query::NamedTable, pgorm_query::TableName"),
        (
            "table.col",
            "pgorm_query::Expr::col, pgorm_query::ColumnRef",
        ),
        ("table.star", "pgorm_query::ColumnRef::TableAsterisk"),
        ("table.as_", "pgorm_query::NamedTable::alias"),
        (
            "table.tablesample",
            "pgorm_query::TableSample::{new, repeatable}, SampleMethod, FromItem::TableSample",
        ),
        (
            "table_function",
            "pgorm_query::TableFunction::{new, rows_from, and, with_ordinality, columns}",
        ),
        (
            "table_function.column_definition",
            "pgorm_query::TableFunction::column_definition",
        ),
        ("table_function.alias", "pgorm_query::TableFunction::alias"),
        (
            "grouping_element",
            "pgorm_query::GroupingElement::{set, empty, rollup, cube, sets}, GroupingSets::add",
        ),
        (
            "from_item.col",
            "pgorm_query::FromItem::qualifier, ColumnRef::TableColumn",
        ),
        (
            "from_item.star",
            "pgorm_query::FromItem::qualifier, ColumnRef::TableAsterisk",
        ),
        ("select", "pgorm_query::Query::select"),
        ("array", "pgorm_query::Expr::array"),
        ("exists", "pgorm_query::Expr::exists"),
        (
            "scalar",
            "pgorm_query::SimpleExpr::SubQuery, SubQueryStatement::SelectStatement",
        ),
        ("any_", "pgorm_query::Expr::any, Func::any"),
        ("all_", "pgorm_query::Expr::all, Func::all"),
        (
            "select.select",
            "pgorm_query::SelectStatement::clear_selects, expr, expr_as",
        ),
        (
            "select.from_",
            "pgorm_query::SelectStatement::from, FromItem",
        ),
        ("select.where", "pgorm_query::SelectStatement::cond_where"),
        (
            "select.join",
            "pgorm_query::SelectStatement::{join, join_lateral}, FromItem",
        ),
        ("select.as_", "pgorm_query::FromItem::SubQuery"),
        (
            "select.lock",
            "pgorm_query::SelectStatement::{lock_with_tables, lock_with_tables_behavior}, LockType, LockBehavior",
        ),
        (
            "select.cross_join",
            "pgorm_query::SelectStatement::cross_join, FromItem",
        ),
        (
            "select.group_by",
            "pgorm_query::SelectStatement::{add_group_by, group_by_element}",
        ),
        (
            "select.group_by_distinct",
            "pgorm_query::SelectStatement::group_by_distinct",
        ),
        ("select.having", "pgorm_query::SelectStatement::cond_having"),
        (
            "select.order_by",
            "pgorm_query::SelectStatement::order_by_expr, order_by_expr_with_nulls",
        ),
        (
            "select.limit",
            "pgorm_query::SelectStatement::limit, reset_limit",
        ),
        (
            "select.limit_with_ties",
            "pgorm_query::SelectStatement::{limit_with_ties, check_tie_limit}",
        ),
        (
            "select.offset",
            "pgorm_query::SelectStatement::offset, reset_offset",
        ),
        ("select.distinct", "pgorm_query::SelectStatement::distinct"),
        (
            "select.distinct_on",
            "pgorm_query::SelectStatement::distinct_on",
        ),
        (
            "select.window",
            "pgorm_query::SelectStatement::{window, check_windows}",
        ),
        (
            "window.extended",
            "pgorm_query::ExtendedWindow::{new, frame}, OrderedStatement::{order_by_expr, order_by_expr_with_nulls}",
        ),
        (
            "window",
            "pgorm_query::WindowStatement::new, OverStatement::add_partition_by, OrderedStatement::{order_by_expr, order_by_expr_with_nulls}, WindowStatement::frame",
        ),
        (
            "window.frame",
            "pgorm_query::FrameType::{unbounded_preceding, preceding, current_row, following}, FrameStart::{and_preceding, and_current_row, and_following, and_unbounded_following, exclude}, FrameClause::exclude, FrameExclusion",
        ),
        (
            "window_function",
            "pgorm_query::Func::named, FunctionCall::args",
        ),
        (
            "select.inspect",
            "pgorm_query::SelectStatement::{check_clauses, build}",
        ),
        (
            "insert",
            "pgorm_query::Query::insert, InsertStatement::into_table",
        ),
        ("insert.columns", "pgorm_query::InsertStatement::columns"),
        ("insert.values", "pgorm_query::InsertStatement::values"),
        ("insert.select", "pgorm_query::InsertStatement::select_from"),
        (
            "insert.overriding",
            "pgorm_query::InsertStatement::overriding, Overriding",
        ),
        ("insert.with_", "pgorm_query::InsertStatement::with"),
        (
            "insert.default_values",
            "pgorm_query::InsertStatement::or_default_values",
        ),
        (
            "insert.on_conflict",
            "pgorm_query::InsertStatement::on_conflict",
        ),
        (
            "insert.returning",
            "pgorm_query::InsertStatement::returning",
        ),
        ("insert.inspect", "pgorm_query::InsertStatement::build"),
        (
            "update",
            "pgorm_query::Query::update, UpdateStatement::table",
        ),
        ("update.set", "pgorm_query::UpdateStatement::value"),
        ("update.where", "pgorm_query::UpdateStatement::cond_where"),
        ("update.from_", "pgorm_query::UpdateStatement::from, FromItem"),
        ("update.with_", "pgorm_query::UpdateStatement::with"),
        (
            "update.all_rows",
            "pgorm_query::UpdateStatement with explicit unrestricted intent",
        ),
        (
            "update.returning",
            "pgorm_query::UpdateStatement::returning",
        ),
        ("update.inspect", "pgorm_query::UpdateStatement::build"),
        (
            "delete",
            "pgorm_query::Query::delete, DeleteStatement::from_table",
        ),
        ("delete.where", "pgorm_query::DeleteStatement::cond_where"),
        ("delete.using", "pgorm_query::DeleteStatement::using, FromItem"),
        ("delete.with_", "pgorm_query::DeleteStatement::with"),
        (
            "delete.all_rows",
            "pgorm_query::DeleteStatement with explicit unrestricted intent",
        ),
        (
            "delete.returning",
            "pgorm_query::DeleteStatement::returning",
        ),
        ("delete.inspect", "pgorm_query::DeleteStatement::build"),
        ("conflict.do_nothing", "pgorm_query::OnConflict::do_nothing"),
        (
            "conflict.on",
            "pgorm_query::OnConflict::{column, expr}, ConflictTarget::{and_column, and_expr}",
        ),
        (
            "conflict.on_constraint",
            "pgorm_query::OnConflict::constraint, ConflictConstraint::{do_nothing, update_column, value}",
        ),
        (
            "conflict.target.where",
            "pgorm_query::ConflictTarget::cond_where",
        ),
        (
            "conflict.target.do_nothing",
            "pgorm_query::ConflictTarget::do_nothing",
        ),
        (
            "conflict.target.update",
            "pgorm_query::ConflictTarget::update_column, ConflictUpdate::update_column",
        ),
        ("conflict.target.set", "pgorm_query::ConflictTarget::value"),
        ("conflict.update.set", "pgorm_query::ConflictUpdate::value"),
        (
            "conflict.update.update",
            "pgorm_query::ConflictUpdate::update_column",
        ),
        (
            "conflict.update.where",
            "pgorm_query::ConflictUpdate::cond_where",
        ),
        (
            "returning.row_versions",
            "pgorm_query::ReturningRow, ColumnRef::{RowColumn, RowAsterisk}",
        ),
        (
            "returning.rename",
            "pgorm_query::ReturningClause::{old_as, new_as}",
        ),
        ("select.with_", "pgorm_query::SelectStatement::with"),
        (
            "select.union",
            "pgorm_query::SelectStatement::union, UnionType::Distinct",
        ),
        (
            "select.union_all",
            "pgorm_query::SelectStatement::union, UnionType::All",
        ),
        (
            "select.intersect",
            "pgorm_query::SelectStatement::union, UnionType::Intersect",
        ),
        (
            "select.intersect_all",
            "pgorm_query::SelectStatement::union, UnionType::IntersectAll",
        ),
        (
            "select.except_",
            "pgorm_query::SelectStatement::union, UnionType::Except",
        ),
        (
            "select.except_all",
            "pgorm_query::SelectStatement::union, UnionType::ExceptAll",
        ),
        (
            "with",
            "pgorm_query::WithClause::{new, cte}, CommonTableExpression::{new, column, materialized}",
        ),
        (
            "with.recursive",
            "pgorm_query::RecursiveWithClause::{new, search, cycle}, Search::new, SearchOrder, Cycle::new",
        ),
        ("merge", "pgorm_query::Query::merge, PendingMerge, FromItem"),
        (
            "merge.when_matched",
            "pgorm_query::MergeStatement::{when_matched, when_matched_and}",
        ),
        (
            "merge.when_not_matched",
            "pgorm_query::MergeStatement::{when_not_matched, when_not_matched_and}",
        ),
        (
            "merge.when_not_matched_by_source",
            "pgorm_query::MergeStatement::{when_not_matched_by_source, when_not_matched_by_source_and}",
        ),
        (
            "merge_action.update",
            "pgorm_query::MergeUpdate::{value, and_value}, MatchedAction::Update",
        ),
        (
            "merge_action.insert",
            "pgorm_query::MergeInsert::{value, and_value, overriding}, NotMatchedAction::Insert",
        ),
        ("merge_action.delete", "pgorm_query::MatchedAction::Delete"),
        (
            "merge_action.do_nothing",
            "pgorm_query::MatchedAction::DoNothing, NotMatchedAction::DoNothing",
        ),
        (
            "merge_action.insert_defaults",
            "pgorm_query::NotMatchedAction::InsertDefaultValues",
        ),
        ("merge.returning", "pgorm_query::MergeStatement::returning"),
        (
            "merge.returning_action",
            "pgorm_query::MergeStatement::returning_action",
        ),
        ("merge.with_", "pgorm_query::MergeStatement::with"),
        ("merge.only", "pgorm_query::MergeStatement::only"),
        ("merge.inspect", "pgorm_query::MergeStatement::build"),
        ("raw_sql", "pgorm::SqlText with pgorm_query::Values"),
        ("raw_sql.inline_sql", "pgorm_query::inject_parameters"),
    ]
    .into_iter()
    .map(|(name, rust_api)| (name.into(), json!({"rust_api": rust_api, "features": []})))
    .collect()
}
