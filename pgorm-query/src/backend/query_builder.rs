use crate::keywords::Position;
use crate::query::RowLimit;
use crate::types::LikeEscape;
use crate::{
    extension::{
        ExtensionCreateStatement, ExtensionDropOpt, ExtensionDropStatement, TypeAlterAddOpt,
        TypeAlterOpt, TypeAlterStatement, TypeAs, TypeCreateStatement, TypeDropOpt,
        TypeDropStatement, TypeRef,
    },
    template::Segment,
    *,
};
use std::ops::Deref;

// Construct-specific renderers live in child modules, which see this file's
// private `prepare_*` methods without widening them.
#[path = "query_builder_case.rs"]
mod case;
#[path = "query_builder_collate.rs"]
mod collate;
#[path = "query_builder_composite.rs"]
mod composite;
#[path = "query_builder_constraint.rs"]
mod constraint;
mod copy;
#[path = "query_builder_cte.rs"]
mod cte;
#[path = "query_builder_domain.rs"]
mod domain;
#[path = "query_builder_from_item.rs"]
mod from_item;
#[path = "query_builder_generated.rs"]
mod generated;
#[path = "query_builder_grouping.rs"]
mod grouping;
#[path = "query_builder_json.rs"]
mod json;
#[path = "query_builder_json_table.rs"]
mod json_table;
#[path = "query_builder_merge.rs"]
mod merge;
mod precedence;
#[path = "query_builder_range.rs"]
mod range;
#[path = "query_builder_returning.rs"]
mod returning;
#[path = "query_builder_schema.rs"]
mod schema;
#[path = "query_builder_sequence.rs"]
mod sequence;
#[path = "query_builder_set_operation.rs"]
mod set_operation;
#[path = "query_builder_subscript.rs"]
mod subscript;
#[path = "query_builder_view.rs"]
mod view;
#[path = "query_builder_window.rs"]
mod window;

/// Discard sub-microsecond digits before a temporal value is rendered as a
/// literal. PostgreSQL stores microseconds and would round a ninth digit,
/// while the parameter path discards it — so without this the same value means
/// two different things depending on how it travelled.
macro_rules! truncate_to_microsecond {
    ($rounder:path) => {
        <$rounder>::new()
            .smallest(jiff::Unit::Microsecond)
            .mode(jiff::RoundMode::Trunc)
    };
}

// [spec:pgorm:def:sql.render+1]
// [spec:pgorm:req:sql.render.oracle+4] (the renderer whose every output the oracle in
// pgorm-query/tests/postgres/oracle.rs holds to the libpg_query grammar)
#[derive(Debug, Clone, Copy)]
pub struct QueryBuilder;

impl QueryBuilder {
    fn prepare_select_distinct(&self, select_distinct: &SelectDistinct, sql: &mut dyn SqlWriter) {
        match select_distinct {
            SelectDistinct::All => write!(sql, "ALL").unwrap(),
            SelectDistinct::Distinct => write!(sql, "DISTINCT").unwrap(),
            SelectDistinct::DistinctOn(cols) => {
                write!(sql, "DISTINCT ON (").unwrap();
                cols.iter().fold(true, |first, column_ref| {
                    if !first {
                        write!(sql, ", ").unwrap();
                    }
                    self.prepare_column_ref(column_ref, sql);
                    false
                });
                write!(sql, ")").unwrap();
            }
        };
    }

    // [spec:pgorm:req:sql.render.select-order+7] (order expressions: ASC/DESC, NULLS, Order::Field)
    fn prepare_order_expr(&self, order_expr: &OrderExpr, sql: &mut dyn SqlWriter) {
        if !matches!(order_expr.order, Order::Field(_)) {
            self.prepare_simple_expr(&order_expr.expr, sql);
        }
        self.prepare_order(order_expr, sql);
        match order_expr.nulls {
            None => (),
            Some(NullOrdering::Last) => write!(sql, " NULLS LAST").unwrap(),
            Some(NullOrdering::First) => write!(sql, " NULLS FIRST").unwrap(),
        }
    }

    // [spec:pgorm:req:sql.render.string-escape+1] (single-quote wrapping; E-string when a backslash is present)
    fn write_string_quoted(&self, string: &str, buffer: &mut String) {
        let escaped = self.escape_string(string);
        let string = if escaped.find('\\').is_some() {
            "E'".to_owned() + &escaped + "'"
        } else {
            "'".to_owned() + &escaped + "'"
        };
        write!(buffer, "{string}").unwrap()
    }

    fn write_bytes(&self, bytes: &[u8], buffer: &mut String) {
        write!(buffer, "'\\x").unwrap();
        for b in bytes {
            write!(buffer, "{b:02X}").unwrap();
        }
        write!(buffer, "'").unwrap();
    }

    /// Translate [`InsertStatement`] into SQL statement.
    // [spec:pgorm:req:sql.render.insert+2]
    // [spec:pgorm:sem:query.build.with.attach+1]
    pub(crate) fn prepare_insert_statement(
        &self,
        insert: &InsertStatement,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(with_clause) = &insert.with {
            self.prepare_with_clause(with_clause, sql);
        }

        write!(sql, "INSERT").unwrap();

        if let Some(table) = &insert.table {
            write!(sql, " INTO ").unwrap();
            self.prepare_named_table(table, sql);
        }

        // Grammatical in both source shapes, and written once so neither can
        // drop it silently.
        let overriding = insert.overriding.map(|overriding| match overriding {
            Overriding::SystemValue => "OVERRIDING SYSTEM VALUE",
            Overriding::UserValue => "OVERRIDING USER VALUE",
        });

        if let (Some(num_rows), true, true) = (
            insert.default_values,
            insert.columns.is_empty(),
            insert.source.is_none(),
        ) {
            write!(sql, " ").unwrap();
            if let Some(overriding) = overriding {
                write!(sql, "{overriding} ").unwrap();
            }
            self.insert_default_values(num_rows, sql);
        } else {
            write!(sql, " ").unwrap();
            write!(sql, "(").unwrap();
            insert.columns.iter().fold(true, |first, col| {
                if !first {
                    write!(sql, ", ").unwrap()
                }
                col.prepare(sql.as_writer());
                false
            });
            write!(sql, ")").unwrap();

            if let Some(overriding) = overriding {
                write!(sql, " {overriding}").unwrap();
            }

            if let Some(source) = &insert.source {
                write!(sql, " ").unwrap();
                match source {
                    InsertValueSource::Values(values) => {
                        write!(sql, "VALUES ").unwrap();
                        values.iter().fold(true, |first, row| {
                            if !first {
                                write!(sql, ", ").unwrap()
                            }
                            write!(sql, "(").unwrap();
                            row.iter().fold(true, |first, col| {
                                if !first {
                                    write!(sql, ", ").unwrap()
                                }
                                self.prepare_simple_expr(col, sql);
                                false
                            });
                            write!(sql, ")").unwrap();
                            false
                        });
                    }
                    InsertValueSource::Select(select_query) => {
                        self.prepare_select_statement(select_query.deref(), sql);
                    }
                }
            }
        }

        self.prepare_on_conflict(&insert.on_conflict, sql);

        self.prepare_returning(insert.returning.as_ref(), None, sql);
    }

    /// Translate [`SelectStatement`] into SQL statement.
    // [spec:pgorm:req:sql.render.select-order+7]
    // [spec:pgorm:sem:query.build.with.attach+1]
    pub(crate) fn prepare_select_statement(
        &self,
        select: &SelectStatement,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(with_clause) = &select.with {
            self.prepare_with_clause(with_clause, sql);
        }

        self.open_set_operation_groups(&select.unions, sql);

        write!(sql, "SELECT ").unwrap();

        if let Some(distinct) = &select.distinct {
            self.prepare_select_distinct(distinct, sql);
            write!(sql, " ").unwrap();
        }

        select.selects.iter().fold(true, |first, expr| {
            if !first {
                write!(sql, ", ").unwrap()
            }
            self.prepare_select_expr(expr, sql);
            false
        });

        if !select.from.is_empty() {
            write!(sql, " FROM ").unwrap();
            select.from.iter().fold(true, |first, from_item| {
                if !first {
                    write!(sql, ", ").unwrap()
                }
                self.prepare_from_item(from_item, sql);
                false
            });
        }

        if !select.join.is_empty() {
            for expr in select.join.iter() {
                write!(sql, " ").unwrap();
                self.prepare_join_expr(expr, sql);
            }
        }

        self.prepare_condition(&select.r#where, "WHERE", sql);

        if !select.groups.is_empty() {
            write!(sql, " GROUP BY ").unwrap();
            if select.group_distinct {
                write!(sql, "DISTINCT ").unwrap();
            }
            self.prepare_grouping_list(&select.groups, sql);
        }

        self.prepare_condition(&select.having, "HAVING", sql);

        self.prepare_window_clause(&select.windows, sql);

        self.prepare_set_operations(&select.unions, sql);

        if !select.orders.is_empty() {
            write!(sql, " ORDER BY ").unwrap();
            select.orders.iter().fold(true, |first, expr| {
                if !first {
                    write!(sql, ", ").unwrap()
                }
                self.prepare_order_expr(expr, sql);
                false
            });
        }

        self.prepare_select_limit_offset(select, sql);

        if let Some(lock) = &select.lock {
            write!(sql, " ").unwrap();
            self.prepare_select_lock(lock, sql);
        }
    }

    /// Translate the row limit and OFFSET of a [`SelectStatement`]. A tie
    /// limit takes the place `LIMIT` would, before `OFFSET`, which the
    /// grammar admits on either side of a `FETCH FIRST`.
    // [spec:pgorm:req:sql.render.tie-limit]
    pub(crate) fn prepare_select_limit_offset(
        &self,
        select: &SelectStatement,
        sql: &mut dyn SqlWriter,
    ) {
        match &select.limit {
            Some(RowLimit::Count(limit)) => {
                write!(sql, " LIMIT ").unwrap();
                sql.push_param((*limit).into());
            }
            Some(RowLimit::WithTies(count)) => {
                write!(sql, " FETCH FIRST ").unwrap();
                sql.push_param((*count).into());
                write!(sql, " ROWS WITH TIES").unwrap();
            }
            None => {}
        }

        if let Some(offset) = &select.offset {
            write!(sql, " OFFSET ").unwrap();
            sql.push_param(offset.clone());
        }
    }

    /// Translate [`UpdateStatement`] into SQL statement.
    // [spec:pgorm:req:sql.render.update-delete+3] (UPDATE half)
    // [spec:pgorm:sem:query.build.with.attach+1]
    pub(crate) fn prepare_update_statement(
        &self,
        update: &UpdateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(with_clause) = &update.with {
            self.prepare_with_clause(with_clause, sql);
        }

        write!(sql, "UPDATE ").unwrap();

        if let Some(table) = &update.table {
            self.prepare_named_table(table, sql);
        }

        write!(sql, " SET ").unwrap();

        update.values.iter().fold(true, |first, row| {
            if !first {
                write!(sql, ", ").unwrap()
            }
            let (col, v) = row;
            col.prepare(sql.as_writer());
            write!(sql, " = ").unwrap();
            self.prepare_simple_expr(v, sql);
            false
        });

        if !update.from.is_empty() {
            write!(sql, " FROM ").unwrap();
            update.from.iter().fold(true, |first, from_item| {
                if !first {
                    write!(sql, ", ").unwrap()
                }
                self.prepare_from_item(from_item, sql);
                false
            });
        }

        self.prepare_condition(&update.r#where, "WHERE", sql);

        self.prepare_returning(update.returning.as_ref(), None, sql);
    }

    /// Translate [`DeleteStatement`] into SQL statement.
    // [spec:pgorm:req:sql.render.update-delete+3] (DELETE half)
    // [spec:pgorm:sem:query.build.with.attach+1]
    pub(crate) fn prepare_delete_statement(
        &self,
        delete: &DeleteStatement,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(with_clause) = &delete.with {
            self.prepare_with_clause(with_clause, sql);
        }

        write!(sql, "DELETE ").unwrap();

        if let Some(table) = &delete.table {
            write!(sql, "FROM ").unwrap();
            self.prepare_named_table(table, sql);
        }

        if !delete.using.is_empty() {
            write!(sql, " USING ").unwrap();
            delete.using.iter().fold(true, |first, from_item| {
                if !first {
                    write!(sql, ", ").unwrap()
                }
                self.prepare_from_item(from_item, sql);
                false
            });
        }

        self.prepare_condition(&delete.r#where, "WHERE", sql);

        self.prepare_returning(delete.returning.as_ref(), None, sql);
    }

    // [spec:pgorm:sem:sql.render.empty-in+1]
    // [spec:pgorm:req:sql.render.subquery+5] (SubQuery/Tuple/Values expression arms)
    // [spec:pgorm:req:sql.render.custom-expr+3]
    fn prepare_simple_expr(&self, simple_expr: &SimpleExpr, sql: &mut dyn SqlWriter) {
        match simple_expr {
            SimpleExpr::Column(column_ref) => {
                self.prepare_column_ref(column_ref, sql);
            }
            SimpleExpr::Tuple(exprs) => {
                self.prepare_tuple(exprs, sql);
            }
            SimpleExpr::Unary(op, expr) => {
                self.prepare_un_oper(op, sql);
                write!(sql, " ").unwrap();
                let bare = self.inner_expr_well_known_greater_precedence(expr, &(*op).into());
                self.prepare_operand(expr, bare, sql);
            }
            SimpleExpr::FunctionCall(func) => {
                self.prepare_function_name(&func.func, sql);
                self.prepare_function_arguments(func, sql);
            }
            SimpleExpr::Binary(left, op, right) => match (op, right.as_ref()) {
                (BinOper::In, SimpleExpr::Tuple(t)) if t.is_empty() => self.binary_expr(
                    &SimpleExpr::Value("a".into()),
                    &BinOper::Equal,
                    &SimpleExpr::Value("b".into()),
                    sql,
                ),
                (BinOper::NotIn, SimpleExpr::Tuple(t)) if t.is_empty() => self.binary_expr(
                    &SimpleExpr::Value("a".into()),
                    &BinOper::Equal,
                    &SimpleExpr::Value("a".into()),
                    sql,
                ),
                _ => self.binary_expr(left, op, right, sql),
            },
            SimpleExpr::SubQuery(oper, sel) => {
                if let Some(oper) = oper {
                    self.prepare_sub_query_oper(oper, sql);
                }
                write!(sql, "(").unwrap();
                sel.prepare_statement(sql);
                write!(sql, ")").unwrap();
            }
            // [spec:pgorm:req:sql.render.param-vs-inline+1]
            SimpleExpr::Value(val) => {
                sql.push_param(val.clone());
            }
            SimpleExpr::Values(list) => {
                write!(sql, "(").unwrap();
                list.iter().fold(true, |first, val| {
                    if !first {
                        write!(sql, ", ").unwrap();
                    }
                    sql.push_param(val.clone());
                    false
                });
                write!(sql, ")").unwrap();
            }
            SimpleExpr::Raw(s) => {
                write!(sql, "{s}").unwrap();
            }
            // [spec:pgorm:req:sql.render.custom-expr+3]
            SimpleExpr::Template(template) => {
                for segment in template.segments() {
                    match segment {
                        Segment::Text(text) => write!(sql, "{text}").unwrap(),
                        Segment::Value(expr) => self.prepare_simple_expr(expr, sql),
                    }
                }
            }
            SimpleExpr::Keyword(keyword) => {
                self.prepare_keyword(keyword, sql);
            }
            // THE cast, `CAST(operand AS "type")` — one arm because there is
            // one shape; the type a `TypeName`, so a name is a name and never
            // SQL; a `Value` operand keeps the source-typed placeholder pin of
            // `sql.render.cast-param-type`.
            // [spec:pgorm:req:sql.render.cast-param-type+4]
            // [spec:pgorm:req:sql.ast.cast-shape+2]
            SimpleExpr::AsEnum(type_name, expr) => {
                write!(sql, "CAST(").unwrap();
                match expr.as_ref() {
                    SimpleExpr::Value(value) => sql.push_param_source_typed(value.clone()),
                    other => self.prepare_simple_expr(other, sql),
                }
                write!(sql, " AS {})", type_name.to_sql_string()).unwrap();
            }
            SimpleExpr::Case(case_stmt) => {
                self.prepare_case_statement(case_stmt, sql);
            }
            SimpleExpr::SimpleCase(case_stmt) => {
                self.prepare_simple_case_statement(case_stmt, sql);
            }
            SimpleExpr::Subscript(base, subscript) => {
                self.prepare_subscript(base, subscript, sql);
            }
            SimpleExpr::Collate(operand, collation) => {
                self.prepare_collate(operand, collation, sql);
            }
            // [spec:pgorm:req:sql.render.grouping+1]
            SimpleExpr::Grouping(grouping) => {
                write!(sql, "GROUPING").unwrap();
                self.prepare_tuple(&grouping.args, sql);
            }
            SimpleExpr::Constant(val) => {
                self.prepare_constant(val, sql);
            }
            SimpleExpr::LikePattern(like) => {
                self.prepare_like_expr(like, sql);
            }
            SimpleExpr::SqlJson(json) => {
                self.prepare_sql_json(json, sql);
            }
        }
    }

    /// Translate a [`LikeExpr`] into the pattern and optional `ESCAPE` tail of a
    /// `LIKE` / `ILIKE` / `SIMILAR TO`.
    // [spec:pgorm:def:sql.render.operators+8]
    fn prepare_like_expr(&self, like: &LikeExpr, sql: &mut dyn SqlWriter) {
        sql.push_param(like.pattern.clone().into());
        match like.escape {
            Some(LikeEscape::Char(escape)) => {
                write!(sql, " ESCAPE ").unwrap();
                self.prepare_constant(&escape.into(), sql);
            }
            Some(LikeEscape::Off) => write!(sql, " ESCAPE ''").unwrap(),
            None => {}
        }
    }

    /// Translate [`LockType`] into SQL statement.
    // [spec:pgorm:sem:sql.render.locking+1]
    fn prepare_select_lock(&self, lock: &LockClause, sql: &mut dyn SqlWriter) {
        write!(
            sql,
            "FOR {}",
            match lock.r#type {
                LockType::Update => "UPDATE",
                LockType::NoKeyUpdate => "NO KEY UPDATE",
                LockType::Share => "SHARE",
                LockType::KeyShare => "KEY SHARE",
            }
        )
        .unwrap();
        if !lock.tables.is_empty() {
            write!(sql, " OF ").unwrap();
            lock.tables.iter().fold(true, |first, name| {
                if !first {
                    write!(sql, ", ").unwrap();
                }
                name.prepare(sql.as_writer());
                false
            });
        }
        if let Some(behavior) = lock.behavior {
            match behavior {
                LockBehavior::Nowait => write!(sql, " NOWAIT").unwrap(),
                LockBehavior::SkipLocked => write!(sql, " SKIP LOCKED").unwrap(),
            }
        }
    }

    /// Translate [`SelectExpr`] into SQL statement.
    // [spec:pgorm:req:sql.render.window+7] (OVER attachment: named reference, inline spec, alias)
    fn prepare_select_expr(&self, select_expr: &SelectExpr, sql: &mut dyn SqlWriter) {
        self.prepare_simple_expr(&select_expr.expr, sql);
        match &select_expr.window {
            Some(WindowSelectType::Name(name)) => {
                write!(sql, " OVER ").unwrap();
                name.prepare(sql.as_writer())
            }
            Some(WindowSelectType::Query(window)) => {
                write!(sql, " OVER ").unwrap();
                self.prepare_window_spec(window, sql);
            }
            Some(WindowSelectType::Extended(window)) => {
                write!(sql, " OVER ").unwrap();
                self.prepare_extended_window(window, sql);
            }
            None => {}
        };

        if let Some(alias) = &select_expr.alias {
            write!(sql, " AS ").unwrap();
            alias.prepare(sql.as_writer());
        }
    }

    /// Translate [`JoinExpr`] into SQL statement.
    // [spec:pgorm:req:sql.render.joins+2]
    fn prepare_join_expr(&self, join_expr: &JoinExpr, sql: &mut dyn SqlWriter) {
        match &join_expr.join {
            JoinKind::Cross => write!(sql, "CROSS JOIN").unwrap(),
            JoinKind::Qualified(join_type, _) => self.prepare_join_type(join_type, sql),
        }
        write!(sql, " ").unwrap();
        self.prepare_join_from_item(join_expr, sql);
        if let JoinKind::Qualified(_, on) = &join_expr.join {
            self.prepare_join_on(on, sql);
        }
    }

    fn prepare_join_from_item(&self, join_expr: &JoinExpr, sql: &mut dyn SqlWriter) {
        if join_expr.lateral {
            write!(sql, "LATERAL ").unwrap();
        }
        self.prepare_from_item(&join_expr.table, sql);
    }

    /// Translate [`FromItem`] into SQL statement.
    // [spec:pgorm:req:sql.render.subquery+5] (value-bearing from items carry mandatory aliases)
    fn prepare_from_item(&self, from_item: &FromItem, sql: &mut dyn SqlWriter) {
        match from_item {
            FromItem::Table(table) => self.prepare_named_table(table, sql),
            FromItem::SubQuery(query, alias) => {
                write!(sql, "(").unwrap();
                self.prepare_select_statement(query, sql);
                write!(sql, ")").unwrap();
                write!(sql, " AS ").unwrap();
                alias.prepare(sql.as_writer());
            }
            FromItem::ValuesList(values, alias) => {
                write!(sql, "(").unwrap();
                self.prepare_values_list(values, sql);
                write!(sql, ")").unwrap();
                write!(sql, " AS ").unwrap();
                alias.prepare(sql.as_writer());
            }
            FromItem::FunctionCall(func, alias) => {
                self.prepare_function_name(&func.func, sql);
                self.prepare_function_arguments(func, sql);
                write!(sql, " AS ").unwrap();
                alias.prepare(sql.as_writer());
            }
            // A fragment this crate did not build closes on a line of its own:
            // its last text may end inside a `--` comment, which would
            // otherwise swallow the closing parenthesis. Each resolved value
            // re-enters through `prepare_simple_expr`, so the fragment's own
            // `$N` are renumbered into this statement's parameter space.
            // [spec:pgorm:req:sql.render.subquery+5]
            FromItem::Template(template, alias) => {
                write!(sql, "(").unwrap();
                for segment in template.segments() {
                    match segment {
                        Segment::Text(text) => write!(sql, "{text}").unwrap(),
                        Segment::Value(expr) => self.prepare_simple_expr(expr, sql),
                    }
                }
                write!(sql, "\n) AS ").unwrap();
                alias.prepare(sql.as_writer());
            }
            FromItem::JsonTable(table, alias) => self.prepare_json_table(table, alias, sql),
            FromItem::TableFunction(function, alias) => {
                self.prepare_table_function(function, alias, sql)
            }
            FromItem::TableSample(sample) => self.prepare_table_sample(sample, sql),
        }
    }

    fn prepare_column_ref(&self, column_ref: &ColumnRef, sql: &mut dyn SqlWriter) {
        match column_ref {
            ColumnRef::Column(column) => column.prepare(sql.as_writer()),
            ColumnRef::TableColumn(table, column) => {
                table.prepare(sql.as_writer());
                write!(sql, ".").unwrap();
                column.prepare(sql.as_writer());
            }
            ColumnRef::SchemaTableColumn(schema, table, column) => {
                schema.prepare(sql.as_writer());
                write!(sql, ".").unwrap();
                table.prepare(sql.as_writer());
                write!(sql, ".").unwrap();
                column.prepare(sql.as_writer());
            }
            ColumnRef::Asterisk => {
                write!(sql, "*").unwrap();
            }
            ColumnRef::TableAsterisk(table) => {
                table.prepare(sql.as_writer());
                write!(sql, ".*").unwrap();
            }
            // [spec:pgorm:req:sql.render.returning+4] the relation's keyword, bare
            ColumnRef::RowColumn(row, column) => {
                write!(sql, "{}.", row.keyword()).unwrap();
                column.prepare(sql.as_writer());
            }
            ColumnRef::RowAsterisk(row) => write!(sql, "{}.*", row.keyword()).unwrap(),
        };
    }

    /// Translate [`UnOper`] into SQL statement.
    // [spec:pgorm:def:sql.render.operators+8] (the only unary operator: NOT)
    fn prepare_un_oper(&self, un_oper: &UnOper, sql: &mut dyn SqlWriter) {
        write!(
            sql,
            "{}",
            match un_oper {
                UnOper::Not => "NOT",
            }
        )
        .unwrap();
    }

    // [spec:pgorm:def:sql.render.operators+8]
    fn prepare_bin_oper(&self, bin_oper: &BinOper, sql: &mut dyn SqlWriter) {
        write!(
            sql,
            "{}",
            match bin_oper {
                BinOper::And => "AND",
                BinOper::Or => "OR",
                BinOper::Like => "LIKE",
                BinOper::NotLike => "NOT LIKE",
                BinOper::SimilarTo => "SIMILAR TO",
                BinOper::NotSimilarTo => "NOT SIMILAR TO",
                BinOper::Is => "IS",
                BinOper::IsNot => "IS NOT",
                BinOper::IsDistinctFrom => "IS DISTINCT FROM",
                BinOper::IsNotDistinctFrom => "IS NOT DISTINCT FROM",
                BinOper::In => "IN",
                BinOper::NotIn => "NOT IN",
                BinOper::Between => "BETWEEN",
                BinOper::NotBetween => "NOT BETWEEN",
                BinOper::BetweenSymmetric => "BETWEEN SYMMETRIC",
                BinOper::NotBetweenSymmetric => "NOT BETWEEN SYMMETRIC",
                BinOper::Equal => "=",
                BinOper::NotEqual => "<>",
                BinOper::SmallerThan => "<",
                BinOper::GreaterThan => ">",
                BinOper::SmallerThanOrEqual => "<=",
                BinOper::GreaterThanOrEqual => ">=",
                BinOper::Add => "+",
                BinOper::Sub => "-",
                BinOper::Mul => "*",
                BinOper::Div => "/",
                BinOper::Mod => "%",
                BinOper::LShift => "<<",
                BinOper::RShift => ">>",
                BinOper::As => "AS",
                BinOper::Raw(raw) => raw,
                BinOper::ILike => "ILIKE",
                BinOper::NotILike => "NOT ILIKE",
                BinOper::Matches => "@@",
                BinOper::Contains => "@>",
                BinOper::Contained => "<@",
                BinOper::Concatenate => "||",
                BinOper::Overlap => "&&",
                BinOper::Similarity => "%",
                BinOper::WordSimilarity => "<%",
                BinOper::StrictWordSimilarity => "<<%",
                BinOper::SimilarityDistance => "<->",
                BinOper::WordSimilarityDistance => "<<->",
                BinOper::StrictWordSimilarityDistance => "<<<->",
                BinOper::GetJsonField => "->",
                BinOper::CastJsonField => "->>",
                BinOper::GetJsonPath => "#>",
                BinOper::CastJsonPath => "#>>",
                BinOper::HasJsonKey => "?",
                BinOper::HasAnyJsonKeys => "?|",
                BinOper::HasAllJsonKeys => "?&",
                BinOper::JsonPathExists => "@?",
                BinOper::Regex => "~",
                BinOper::RegexCaseInsensitive => "~*",
                BinOper::NotRegex => "!~",
                BinOper::NotRegexCaseInsensitive => "!~*",
                BinOper::NotExtendRightOf => "&<",
                BinOper::NotExtendLeftOf => "&>",
                BinOper::AdjacentTo => "-|-",
                BinOper::AtTimeZone => "AT TIME ZONE",
                BinOper::EuclideanDistance => "<->",
                BinOper::NegativeInnerProduct => "<#>",
                BinOper::CosineDistance => "<=>",
            }
        )
        .unwrap();
    }

    /// Translate [`SubQueryOper`] into SQL statement.
    // [spec:pgorm:req:sql.render.subquery+5]
    fn prepare_sub_query_oper(&self, oper: &SubQueryOper, sql: &mut dyn SqlWriter) {
        write!(
            sql,
            "{}",
            match oper {
                SubQueryOper::Exists => "EXISTS",
                SubQueryOper::Any => "ANY",
                SubQueryOper::Some => "SOME",
                SubQueryOper::All => "ALL",
                SubQueryOper::Array => "ARRAY",
            }
        )
        .unwrap();
    }

    /// Translate [`Function`] into SQL statement.
    fn prepare_function_name(&self, function: &Function, sql: &mut dyn SqlWriter) {
        if let Function::Named(iden) = function {
            // [spec:pgorm:req:sql.render.ident-quoting+7]
            let mut name = String::new();
            TypeName::prepare_part(iden, Position::Function, &mut name);
            write!(sql, "{name}").unwrap();
        } else {
            write!(
                sql,
                "{}",
                match function {
                    Function::Max => "MAX",
                    Function::Min => "MIN",
                    Function::Sum => "SUM",
                    Function::Avg => "AVG",
                    Function::PercentileCont => "PERCENTILE_CONT",
                    Function::PercentileDisc => "PERCENTILE_DISC",
                    Function::Abs => "ABS",
                    Function::Coalesce => "COALESCE",
                    Function::Count => "COUNT",
                    Function::IfNull => "COALESCE",
                    Function::CharLength => "CHAR_LENGTH",
                    Function::Lower => "LOWER",
                    Function::Upper => "UPPER",
                    Function::BitAnd => "BIT_AND",
                    Function::BitOr => "BIT_OR",
                    Function::Named(_) => "",
                    Function::Random => "RANDOM",
                    Function::Round => "ROUND",
                    Function::ToTsquery => "TO_TSQUERY",
                    Function::ToTsvector => "TO_TSVECTOR",
                    Function::PhrasetoTsquery => "PHRASETO_TSQUERY",
                    Function::PlaintoTsquery => "PLAINTO_TSQUERY",
                    Function::WebsearchToTsquery => "WEBSEARCH_TO_TSQUERY",
                    Function::TsRank => "TS_RANK",
                    Function::TsRankCd => "TS_RANK_CD",
                    Function::StartsWith => "STARTS_WITH",
                    Function::GenRandomUUID => "GEN_RANDOM_UUID",
                    Function::UuidV4 => "UUIDV4",
                    Function::UuidV7 => "UUIDV7",
                    Function::UuidExtractTimestamp => "UUID_EXTRACT_TIMESTAMP",
                    Function::UuidExtractVersion => "UUID_EXTRACT_VERSION",
                    Function::GenerateSeries => "GENERATE_SERIES",
                    Function::Unnest => "UNNEST",
                    Function::JsonToRecord => "JSON_TO_RECORD",
                    Function::JsonbToRecord => "JSONB_TO_RECORD",
                    Function::JsonToRecordset => "JSON_TO_RECORDSET",
                    Function::JsonbToRecordset => "JSONB_TO_RECORDSET",
                    Function::Any => "ANY",
                    Function::Some => "SOME",
                    Function::All => "ALL",
                }
            )
            .unwrap();
        }
    }

    /// Translate a [`FunctionCall`]'s argument list and the aggregate
    /// modifiers that follow it.
    // [spec:pgorm:req:sql.render.func-mods]
    fn prepare_function_arguments(&self, func: &FunctionCall, sql: &mut dyn SqlWriter) {
        write!(sql, "(").unwrap();
        for (i, expr) in func.args.iter().enumerate() {
            if i != 0 {
                write!(sql, ", ").unwrap();
            }
            if func.mods[i].distinct {
                write!(sql, "DISTINCT ").unwrap();
            }
            self.prepare_simple_expr(expr, sql);
        }
        write!(sql, ")").unwrap();

        let Some(aggregate) = &func.aggregate else {
            return;
        };

        if !aggregate.within_group.is_empty() {
            write!(sql, " WITHIN GROUP (ORDER BY ").unwrap();
            aggregate.within_group.iter().fold(true, |first, expr| {
                if !first {
                    write!(sql, ", ").unwrap()
                }
                self.prepare_order_expr(expr, sql);
                false
            });
            write!(sql, ")").unwrap();
        }

        if let Some(condition) = &aggregate.filter {
            write!(sql, " FILTER (WHERE ").unwrap();
            self.prepare_condition_where(condition, sql);
            write!(sql, ")").unwrap();
        }
    }

    // [spec:pgorm:req:sql.render.cte+5]
    pub(crate) fn prepare_with_clause(&self, with_clause: &AnyWithClause, sql: &mut dyn SqlWriter) {
        match with_clause {
            AnyWithClause::Plain(plain) => self.prepare_plain_with_clause(plain, sql),
            AnyWithClause::Recursive(recursive) => {
                write!(sql, "WITH RECURSIVE ").unwrap();
                self.prepare_with_query_clause_common_table(&recursive.cte, sql);
                self.prepare_with_clause_recursive_options(recursive, sql);
            }
        }
    }

    fn prepare_with_clause_recursive_options(
        &self,
        with_clause: &RecursiveWithClause,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(search) = &with_clause.search {
            write!(
                sql,
                "SEARCH {} FIRST BY ",
                match &search.order {
                    SearchOrder::BREADTH => "BREADTH",
                    SearchOrder::DEPTH => "DEPTH",
                }
            )
            .unwrap();

            self.prepare_simple_expr(&search.expr, sql);

            write!(sql, " SET ").unwrap();

            search.alias.prepare(sql.as_writer());
            write!(sql, " ").unwrap();
        }
        if let Some(cycle) = &with_clause.cycle {
            write!(sql, "CYCLE ").unwrap();

            self.prepare_simple_expr(&cycle.expr, sql);

            write!(sql, " SET ").unwrap();

            cycle.set_as.prepare(sql.as_writer());
            write!(sql, " USING ").unwrap();
            cycle.using.prepare(sql.as_writer());
            write!(sql, " ").unwrap();
        }
    }

    fn prepare_with_query_clause_common_table(
        &self,
        cte: &CommonTableExpression,
        sql: &mut dyn SqlWriter,
    ) {
        cte.table_name.prepare(sql.as_writer());

        if cte.cols.is_empty() {
            write!(sql, " ").unwrap();
        } else {
            write!(sql, " (").unwrap();

            let mut col_first = true;
            for col in &cte.cols {
                if !col_first {
                    write!(sql, ", ").unwrap();
                }
                col_first = false;
                col.prepare(sql.as_writer());
            }

            write!(sql, ") ").unwrap();
        }

        write!(sql, "AS ").unwrap();

        self.prepare_with_query_clause_materialization(cte, sql);

        write!(sql, "(").unwrap();

        cte.query.prepare_statement(sql);

        write!(sql, ") ").unwrap();
    }

    fn prepare_with_query_clause_materialization(
        &self,
        cte: &CommonTableExpression,
        sql: &mut dyn SqlWriter,
    ) {
        match cte.materialized {
            Some(true) => write!(sql, "MATERIALIZED ").unwrap(),
            Some(false) => write!(sql, "NOT MATERIALIZED ").unwrap(),
            None => {}
        }
    }

    /// Translate [`JoinType`] into SQL statement.
    fn prepare_join_type(&self, join_type: &JoinType, sql: &mut dyn SqlWriter) {
        write!(
            sql,
            "{}",
            match join_type {
                JoinType::Join => "JOIN",
                JoinType::InnerJoin => "INNER JOIN",
                JoinType::LeftJoin => "LEFT JOIN",
                JoinType::RightJoin => "RIGHT JOIN",
                JoinType::FullOuterJoin => "FULL OUTER JOIN",
            }
        )
        .unwrap()
    }

    /// Translate [`JoinOn`] into SQL statement.
    // [spec:pgorm:req:sql.render.joins+2]
    fn prepare_join_on(&self, join_on: &JoinOn, sql: &mut dyn SqlWriter) {
        match join_on {
            JoinOn::Condition(c) => self.prepare_condition(c, "ON", sql),
        }
    }

    /// Translate [`Order`] into SQL statement.
    fn prepare_order(&self, order_expr: &OrderExpr, sql: &mut dyn SqlWriter) {
        match &order_expr.order {
            Order::Asc => write!(sql, " ASC").unwrap(),
            Order::Desc => write!(sql, " DESC").unwrap(),
            Order::Field(values) => self.prepare_field_order(order_expr, values, sql),
        }
    }

    /// Translate [`Order::Field`] into SQL statement
    fn prepare_field_order(
        &self,
        order_expr: &OrderExpr,
        values: &Values,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CASE ").unwrap();
        let mut i = 0;
        for value in &values.0 {
            write!(sql, "WHEN ").unwrap();
            self.prepare_simple_expr(&order_expr.expr, sql);
            write!(sql, "=").unwrap();
            let value = self.value_to_string(value);
            write!(sql, "{value}").unwrap();
            write!(sql, " THEN {i} ").unwrap();
            i += 1;
        }
        write!(sql, "ELSE {i} END").unwrap();
    }

    /// Write [`Value`] inline.
    // [spec:pgorm:req:sql.render.param-vs-inline+1] (Constant is always rendered inline)
    fn prepare_constant(&self, value: &Value, sql: &mut dyn SqlWriter) {
        let string = self.value_to_string(value);
        write!(sql, "{string}").unwrap();
    }

    /// Translate a `&[ValueTuple]` into a VALUES list.
    fn prepare_values_list(&self, value_tuples: &[ValueTuple], sql: &mut dyn SqlWriter) {
        write!(sql, "VALUES ").unwrap();
        value_tuples.iter().fold(true, |first, value_tuple| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            write!(sql, "(").unwrap();
            value_tuple.clone().into_iter().fold(true, |first, value| {
                if !first {
                    write!(sql, ", ").unwrap();
                }
                sql.push_param(value);
                false
            });

            write!(sql, ")").unwrap();
            false
        });
    }

    /// Translate [`SimpleExpr::Tuple`] into SQL statement.
    fn prepare_tuple(&self, exprs: &[SimpleExpr], sql: &mut dyn SqlWriter) {
        write!(sql, "(").unwrap();
        for (i, expr) in exprs.iter().enumerate() {
            if i != 0 {
                write!(sql, ", ").unwrap();
            }
            self.prepare_simple_expr(expr, sql);
        }
        write!(sql, ")").unwrap();
    }

    /// Translate [`Keyword`] into SQL statement.
    fn prepare_keyword(&self, keyword: &Keyword, sql: &mut dyn SqlWriter) {
        match keyword {
            Keyword::Null => write!(sql, "NULL").unwrap(),
            Keyword::CurrentDate => write!(sql, "CURRENT_DATE").unwrap(),
            Keyword::CurrentTime => write!(sql, "CURRENT_TIME").unwrap(),
            Keyword::CurrentTimestamp => write!(sql, "CURRENT_TIMESTAMP").unwrap(),
            // [spec:pgorm:req:sql.ddl.domain+3]
            Keyword::Value => write!(sql, "VALUE").unwrap(),
        }
    }

    /// Convert a SQL value into syntax-specific string
    // [spec:pgorm:sem:sql.value.render+2]
    // [spec:pgorm:def:sql.render.value-literals+5]
    pub(crate) fn value_to_string(&self, v: &Value) -> String {
        let mut s = String::new();
        match v {
            Value::Bool(None)
            | Value::TinyInt(None)
            | Value::SmallInt(None)
            | Value::Int(None)
            | Value::BigInt(None)
            | Value::Unsigned(None)
            | Value::BigUnsigned(None)
            | Value::Float(None)
            | Value::Double(None)
            | Value::String(None)
            | Value::Char(None)
            | Value::Bytes(None) => write!(s, "NULL").unwrap(),
            Value::Json(None) => write!(s, "NULL").unwrap(),
            Value::Date(None) => write!(s, "NULL").unwrap(),
            Value::Time(None) => write!(s, "NULL").unwrap(),
            Value::DateTime(None) => write!(s, "NULL").unwrap(),
            Value::DateTimeWithTimeZone(None) => write!(s, "NULL").unwrap(),
            Value::Decimal(None) => write!(s, "NULL").unwrap(),
            Value::Uuid(None) => write!(s, "NULL").unwrap(),
            Value::IpNetwork(None) => write!(s, "NULL").unwrap(),
            Value::MacAddress(None) => write!(s, "NULL").unwrap(),
            Value::Interval(None) => write!(s, "NULL").unwrap(),
            Value::Range(_, None) | Value::Multirange(_, None) => write!(s, "NULL").unwrap(),
            Value::Array(_, None) => write!(s, "NULL").unwrap(),
            Value::Vector(None) => write!(s, "NULL").unwrap(),
            Value::Bool(Some(b)) => write!(s, "{}", if *b { "TRUE" } else { "FALSE" }).unwrap(),
            Value::TinyInt(Some(v)) => write!(s, "{v}").unwrap(),
            Value::SmallInt(Some(v)) => write!(s, "{v}").unwrap(),
            Value::Int(Some(v)) => write!(s, "{v}").unwrap(),
            Value::BigInt(Some(v)) => write!(s, "{v}").unwrap(),
            Value::Unsigned(Some(v)) => write!(s, "{v}").unwrap(),
            Value::BigUnsigned(Some(v)) => write!(s, "{v}").unwrap(),
            Value::Float(Some(v)) => write!(s, "{v}").unwrap(),
            Value::Double(Some(v)) => write!(s, "{v}").unwrap(),
            Value::String(Some(v)) => self.write_string_quoted(v, &mut s),
            Value::Char(Some(v)) => self.write_string_quoted(v.encode_utf8(&mut [0u8; 4]), &mut s),
            Value::Bytes(Some(v)) => self.write_bytes(v, &mut s),
            Value::Json(Some(v)) => self.write_string_quoted(&v.to_string(), &mut s),
            // Each format string asks only for fields its own payload holds.
            // `strftime` formats leniently: a directive the value cannot fill
            // is copied into the output verbatim, so `%:z` against a civil type
            // would render the literal `'... %:z'` — well-formed Rust, malformed
            // SQL, and no error anywhere. Only the instant carries an offset.
            //
            // `%.f` writes the fractional second and omits both it and the dot
            // when it is zero, so a whole second renders exactly as it always
            // has. The value is truncated to microseconds first because that is
            // what PostgreSQL stores and what the parameter path does with the
            // same value; left alone, a ninth digit would be rounded by the
            // server on the literal path and discarded on the bound one, which
            // is the disagreement this rendering exists to avoid.
            Value::Date(Some(v)) => write!(s, "'{}'", v.strftime("%Y-%m-%d")).unwrap(),
            Value::Time(Some(v)) => {
                let v = v
                    .round(truncate_to_microsecond!(jiff::civil::TimeRound))
                    .unwrap_or(**v);
                write!(s, "'{}'", v.strftime("%H:%M:%S%.f")).unwrap()
            }
            Value::DateTime(Some(v)) => {
                let v = v
                    .round(truncate_to_microsecond!(jiff::civil::DateTimeRound))
                    .unwrap_or(**v);
                write!(s, "'{}'", v.strftime("%Y-%m-%d %H:%M:%S%.f")).unwrap()
            }
            Value::DateTimeWithTimeZone(Some(v)) => {
                let v = v
                    .round(truncate_to_microsecond!(jiff::TimestampRound))
                    .unwrap_or(**v);
                write!(s, "'{}'", v.strftime("%Y-%m-%d %H:%M:%S%.f %:z")).unwrap()
            }
            Value::Decimal(Some(v)) => write!(s, "{v}").unwrap(),
            Value::Uuid(Some(v)) => write!(s, "'{v}'").unwrap(),
            Value::Array(ty, Some(v)) if v.is_empty() => match ty.source_type_name() {
                Some(element) => write!(s, "ARRAY []::{element}[]").unwrap(),
                None => write!(s, "ARRAY []").unwrap(),
            },
            Value::Array(_, Some(v)) => write!(
                s,
                "ARRAY [{}]",
                v.iter()
                    .map(|element| self.value_to_string(element))
                    .collect::<Vec<String>>()
                    .join(",")
            )
            .unwrap(),
            Value::Vector(Some(v)) => {
                write!(s, "'[").unwrap();
                for (i, &element) in v.as_slice().iter().enumerate() {
                    if i != 0 {
                        write!(s, ",").unwrap();
                    }
                    write!(s, "{element}").unwrap();
                }
                write!(s, "]'").unwrap();
            }
            Value::IpNetwork(Some(v)) => write!(s, "'{v}'").unwrap(),
            Value::MacAddress(Some(v)) => write!(s, "'{v}'").unwrap(),
            // Cast, unlike the temporal literals: an untyped string is `text`
            // wherever no column types it, and an array of them is a `text[]`
            // no `interval[]` column takes.
            // [spec:pgorm:sem:sql.value.interval-text+1]
            Value::Interval(Some(v)) => {
                self.write_string_quoted(&v.to_string(), &mut s);
                write!(s, "::interval").unwrap()
            }
            Value::Range(ty, Some(v)) => self.write_range(*ty, v, &mut s),
            Value::Multirange(ty, Some(v)) => self.write_multirange(*ty, v, &mut s),
        };
        s
    }

    /// Write ON CONFLICT expression
    // [spec:pgorm:req:sql.render.on-conflict+2]
    fn prepare_on_conflict(&self, on_conflict: &Option<OnConflict>, sql: &mut dyn SqlWriter) {
        let Some(on_conflict) = on_conflict else {
            return;
        };
        write!(sql, " ON CONFLICT").unwrap();
        match on_conflict {
            OnConflict::AnyDoNothing => write!(sql, " DO NOTHING").unwrap(),
            OnConflict::Targeted { arbiter, action } => {
                match arbiter {
                    ConflictArbiter::Inference(target) => {
                        self.prepare_on_conflict_target(target, sql)
                    }
                    ConflictArbiter::Constraint(name) => {
                        write!(sql, " ON CONSTRAINT ").unwrap();
                        name.prepare(sql.as_writer());
                    }
                }
                self.prepare_on_conflict_action(action, sql);
            }
        }
    }

    /// Write ON CONFLICT target
    fn prepare_on_conflict_target(&self, target: &ConflictTarget, sql: &mut dyn SqlWriter) {
        write!(sql, " (").unwrap();
        target.elements().fold(true, |first, element| {
            if !first {
                write!(sql, ", ").unwrap()
            }
            match element {
                ConflictElement::Column(col) => {
                    col.prepare(sql.as_writer());
                }
                ConflictElement::Expr(expr) => {
                    self.prepare_simple_expr(expr, sql);
                }
            }
            false
        });
        write!(sql, ")").unwrap();
        self.prepare_on_conflict_condition(&target.filter, sql);
    }

    /// Write ON CONFLICT action
    fn prepare_on_conflict_action(&self, action: &ConflictAction, sql: &mut dyn SqlWriter) {
        match action {
            ConflictAction::DoNothing => write!(sql, " DO NOTHING").unwrap(),
            ConflictAction::Update { sets, filter } => {
                write!(sql, " DO UPDATE SET ").unwrap();
                sets.iter().fold(true, |first, assignment| {
                    if !first {
                        write!(sql, ", ").unwrap()
                    }
                    match assignment {
                        ConflictAssignment::Column(col) => {
                            col.prepare(sql.as_writer());
                            write!(sql, " = ").unwrap();
                            self.prepare_on_conflict_excluded_table(col, sql);
                        }
                        ConflictAssignment::Expr(col, expr) => {
                            col.prepare(sql.as_writer());
                            write!(sql, " = ").unwrap();
                            self.prepare_simple_expr(expr, sql);
                        }
                    }
                    false
                });
                self.prepare_on_conflict_condition(filter, sql);
            }
        }
    }

    /// Write ON CONFLICT update action by retrieving value from the excluded table
    fn prepare_on_conflict_excluded_table(&self, col: &Name, sql: &mut dyn SqlWriter) {
        write!(sql, "\"excluded\".").unwrap();
        col.prepare(sql.as_writer());
    }

    /// Write ON CONFLICT conditions
    fn prepare_on_conflict_condition(&self, filter: &Option<Condition>, sql: &mut dyn SqlWriter) {
        if let Some(condition) = filter {
            write!(sql, " WHERE ").unwrap();
            self.prepare_condition_where(condition, sql);
        }
    }

    /// Translate a condition to a "WHERE" clause.
    // [spec:pgorm:req:sql.render.condition-chain+1]
    fn prepare_condition(
        &self,
        condition: &ConditionHolder,
        keyword: &str,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(c) = &condition.contents {
            write!(sql, " {keyword} ").unwrap();
            self.prepare_condition_where(c, sql);
        }
    }

    /// Translate part of a condition to part of a "WHERE" clause.
    fn prepare_condition_where(&self, condition: &Condition, sql: &mut dyn SqlWriter) {
        let simple_expr = condition.to_simple_expr();
        self.prepare_simple_expr(&simple_expr, sql);
    }

    /// Write insert default rows expression.
    fn insert_default_values(&self, num_rows: u32, sql: &mut dyn SqlWriter) {
        write!(sql, "VALUES ").unwrap();
        (0..num_rows).fold(true, |first, _| {
            if !first {
                write!(sql, ", ").unwrap()
            }
            write!(sql, "(DEFAULT)").unwrap();
            false
        });
    }

    /// Write TRUE constant
    pub fn prepare_constant_true(&self, sql: &mut dyn SqlWriter) {
        self.prepare_constant(&true.into(), sql);
    }

    /// Write FALSE constant
    pub fn prepare_constant_false(&self, sql: &mut dyn SqlWriter) {
        self.prepare_constant(&false.into(), sql);
    }

    // [spec:pgorm:sem:sql.ddl.panics+4]
    // [spec:pgorm:def:sql.render.ddl.types+7] (serial family for auto-increment columns)
    fn prepare_column_auto_increment(&self, column_type: &ColumnType, sql: &mut dyn SqlWriter) {
        match column_type.serial_spelling() {
            Some(serial) => write!(sql, "{serial}").unwrap(),
            None => self.prepare_column_type(column_type, sql),
        }
    }

    fn prepare_column_type_check_auto_increment(
        &self,
        column_def: &ColumnDef,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(column_type) = &column_def.types {
            let is_auto_increment = column_def
                .spec
                .iter()
                .position(|s| matches!(s, ColumnSpec::AutoIncrement));
            if is_auto_increment.is_some() {
                write!(sql, " ").unwrap();
                self.prepare_column_auto_increment(column_type, sql);
            } else {
                write!(sql, " ").unwrap();
                self.prepare_column_type(column_type, sql);
            }
        }
    }

    /// Write a column definition: its name, then its type as `write_type`
    /// spells it, then every spec that has a spelling of its own. The type is
    /// a callback because `CREATE TABLE` and `ALTER TABLE ADD COLUMN` write it
    /// differently; everything around it is the same in both.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    fn prepare_column_def_parts<F>(
        &self,
        column_def: &ColumnDef,
        sql: &mut dyn SqlWriter,
        write_type: F,
    ) where
        F: Fn(&ColumnDef, &mut dyn SqlWriter),
    {
        column_def.name.prepare(sql.as_writer());

        write_type(column_def, sql);
        self.prepare_column_collation(column_def, sql);

        for column_spec in column_def.spec.iter() {
            if let ColumnSpec::AutoIncrement = column_spec {
                continue;
            }
            if let ColumnSpec::Comment(_) = column_spec {
                continue;
            }
            write!(sql, " ").unwrap();
            self.prepare_column_spec(column_spec, sql);
        }
    }

    fn prepare_column_def(&self, column_def: &ColumnDef, sql: &mut dyn SqlWriter) {
        self.prepare_column_def_parts(column_def, sql, |column_def, sql| {
            self.prepare_column_type_check_auto_increment(column_def, sql);
        });
    }

    // [spec:pgorm:req:sql.ddl.column-types+7]
    // [spec:pgorm:def:sql.render.ddl.types+7]
    fn prepare_column_type(&self, column_type: &ColumnType, sql: &mut dyn SqlWriter) {
        write!(
            sql,
            "{}",
            match column_type {
                // A bare `char` is `char(1)` to PostgreSQL; the type with
                // no length limit is spelled `bpchar`.
                ColumnType::Char(length) => match length {
                    Some(length) => format!("char({length})"),
                    None => "bpchar".into(),
                },
                ColumnType::String(length) => match length {
                    StringLen::N(length) => format!("varchar({length})"),
                    _ => "varchar".into(),
                },
                ColumnType::Text => "text".into(),
                ColumnType::SmallInteger => "smallint".into(),
                ColumnType::Integer => "integer".into(),
                ColumnType::BigInteger => "bigint".into(),
                ColumnType::Float => "real".into(),
                ColumnType::Double => "double precision".into(),
                ColumnType::Decimal(precision) => match precision {
                    Some((precision, scale)) => format!("decimal({precision}, {scale})"),
                    None => "decimal".into(),
                },
                ColumnType::Timestamp => "timestamp".into(),
                ColumnType::TimestampWithTimeZone => "timestamp with time zone".into(),
                ColumnType::Time => "time".into(),
                ColumnType::Date => "date".into(),
                ColumnType::Interval(spec) => {
                    let mut typ = "interval".to_string();
                    match spec {
                        IntervalSpec::Any(None) => {}
                        IntervalSpec::Any(Some(precision)) => {
                            write!(typ, "({precision})").unwrap();
                        }
                        IntervalSpec::Fields(fields) => write!(typ, " {fields}").unwrap(),
                    }
                    typ
                }
                ColumnType::Bytea => "bytea".into(),
                ColumnType::Bit(length) => {
                    match length {
                        Some(length) => format!("bit({length})"),
                        None => "bit".into(),
                    }
                }
                ColumnType::VarBit(length) => {
                    format!("varbit({length})")
                }
                ColumnType::Boolean => "bool".into(),
                ColumnType::Money => "money".into(),
                ColumnType::Json => "json".into(),
                ColumnType::JsonBinary => "jsonb".into(),
                ColumnType::Uuid => "uuid".into(),
                ColumnType::Array(elem_type) => {
                    let mut sql = String::new();
                    self.prepare_column_type(elem_type, &mut sql);
                    format!("{sql}[]")
                }
                ColumnType::Vector(size) => match size {
                    Some(size) => format!("vector({size})"),
                    None => "vector".into(),
                },
                // [spec:pgorm:req:sql.render.ident-quoting+7]
                ColumnType::Named(type_name) => type_name.to_sql_string(),
                ColumnType::Enum { name, schema, .. }
                | ColumnType::CreatedRange { name, schema, .. }
                | ColumnType::CreatedMultirange { name, schema, .. }
                | ColumnType::Domain { name, schema, .. } => {
                    let mut type_name = TypeName::new(Name::clone(name));
                    type_name.schema = schema.clone();
                    type_name.to_sql_string()
                }
                ColumnType::Cidr => "cidr".into(),
                ColumnType::Inet => "inet".into(),
                ColumnType::MacAddr => "macaddr".into(),
                ColumnType::LTree => "ltree".into(),
                ColumnType::Range(ty) => ty.range_type_name().into(),
                ColumnType::Multirange(ty) => ty.multirange_type_name().into(),
            }
        )
        .unwrap()
    }

    // [spec:pgorm:req:sql.ddl.alter-table+13]
    pub(crate) fn prepare_table_alter_statement(
        &self,
        alter: &TableAlterStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER TABLE ").unwrap();
        self.prepare_table_name(&alter.table, sql);
        write!(sql, " ").unwrap();

        alter.options.iter().fold(true, |first, option| {
            if !first {
                write!(sql, ", ").unwrap();
            };
            match option {
                TableAlterOption::AddColumn(AddColumnOption {
                    column,
                    if_not_exists,
                }) => {
                    write!(sql, "ADD COLUMN ").unwrap();
                    if *if_not_exists {
                        write!(sql, "IF NOT EXISTS ").unwrap();
                    }
                    self.prepare_column_def_parts(column, sql, |column_def, sql| {
                        if let Some(column_type) = &column_def.types {
                            write!(sql, " ").unwrap();
                            if column_def
                                .spec
                                .iter()
                                .any(|v| matches!(v, ColumnSpec::AutoIncrement))
                            {
                                self.prepare_column_auto_increment(column_type, sql);
                            } else {
                                self.prepare_column_type(column_type, sql);
                            }
                        }
                    });
                }
                TableAlterOption::ModifyColumn(column_def) => {
                    if let Some(column_type) = &column_def.types {
                        write!(sql, "ALTER COLUMN ").unwrap();
                        column_def.name.prepare(sql.as_writer());
                        write!(sql, " TYPE ").unwrap();
                        self.prepare_column_type(column_type, sql);
                        self.prepare_column_collation(column_def, sql);
                    }
                    let first = column_def.types.is_none();

                    column_def.spec.iter().fold(first, |first, column_spec| {
                        if !first
                            && !matches!(
                                column_spec,
                                ColumnSpec::AutoIncrement | ColumnSpec::Generated { .. }
                            )
                        {
                            write!(sql, ", ").unwrap();
                        }
                        match column_spec {
                            ColumnSpec::AutoIncrement => {}
                            ColumnSpec::Null => {
                                write!(sql, "ALTER COLUMN ").unwrap();
                                column_def.name.prepare(sql.as_writer());
                                write!(sql, " DROP NOT NULL").unwrap();
                            }
                            ColumnSpec::NotNull {
                                name: None,
                                no_inherit: false,
                            } => {
                                write!(sql, "ALTER COLUMN ").unwrap();
                                column_def.name.prepare(sql.as_writer());
                                write!(sql, " SET NOT NULL").unwrap()
                            }
                            // [spec:pgorm:req:sql.ddl.alter-table+13]
                            ColumnSpec::NotNull { name, no_inherit } => {
                                let mut constraint =
                                    NotNullConstraint::new(column_def.name.clone());
                                constraint.name.clone_from(name);
                                constraint.no_inherit = *no_inherit;
                                self.prepare_add_not_null(&constraint, sql);
                            }
                            ColumnSpec::Default(v) => {
                                write!(sql, "ALTER COLUMN ").unwrap();
                                column_def.name.prepare(sql.as_writer());
                                write!(sql, " SET DEFAULT ").unwrap();
                                self.prepare_simple_expr(v, sql);
                            }
                            // A modified column's CHECK is a constraint the
                            // table gains, which ALTER TABLE spells as an ADD.
                            // [spec:pgorm:req:sql.ddl.alter-table+13]
                            ColumnSpec::Check(check) => {
                                write!(sql, "ADD ").unwrap();
                                self.prepare_check_constraint(check, sql);
                            }
                            ColumnSpec::Generated { .. } => {}
                            // `ALTER TABLE` spells identity as an action on the
                            // column, not as a clause of it.
                            // [spec:pgorm:req:sql.ddl.column-def+13]
                            ColumnSpec::Identity(generation, options) => {
                                write!(sql, "ALTER COLUMN ").unwrap();
                                column_def.name.prepare(sql.as_writer());
                                write!(sql, " ADD GENERATED {} AS IDENTITY", generation.keyword())
                                    .unwrap();
                                self.prepare_identity_options(options.as_ref(), sql);
                            }
                            ColumnSpec::RawSuffix(sql_text) => write!(sql, "{sql_text}").unwrap(),
                            ColumnSpec::Comment(_) => {}
                        }
                        false
                    });
                }
                TableAlterOption::DropColumn(column_name) => {
                    write!(sql, "DROP COLUMN ").unwrap();
                    column_name.prepare(sql.as_writer());
                }
                TableAlterOption::DropConstraint(drop) => {
                    self.prepare_drop_constraint(drop, sql);
                }
                TableAlterOption::AddForeignKey {
                    foreign_key,
                    not_valid,
                } => {
                    let create = ForeignKeyCreateStatement {
                        foreign_key: foreign_key.to_owned(),
                    };
                    self.prepare_foreign_key_create_statement_internal(
                        &create,
                        sql,
                        Mode::TableAlter,
                    );
                    self.prepare_not_valid(*not_valid, sql);
                }
                // [spec:pgorm:req:sql.ddl.alter-table+13]
                TableAlterOption::AddPrimaryKey(key) => {
                    write!(sql, "ADD ").unwrap();
                    self.prepare_table_key("PRIMARY KEY", key, sql);
                }
                TableAlterOption::AddUnique(key) => {
                    write!(sql, "ADD ").unwrap();
                    self.prepare_table_key("UNIQUE", key, sql);
                }
                TableAlterOption::SetExpression { column, expr } => {
                    self.prepare_set_expression(column, expr, sql);
                }
                TableAlterOption::DropExpression { column, if_exists } => {
                    self.prepare_drop_expression(column, *if_exists, sql);
                }
                TableAlterOption::AddNotNull(constraint) => {
                    self.prepare_add_not_null(constraint, sql);
                }
                TableAlterOption::AddCheck { check, not_valid } => {
                    write!(sql, "ADD ").unwrap();
                    self.prepare_check_constraint(check, sql);
                    self.prepare_not_valid(*not_valid, sql);
                }
                TableAlterOption::ValidateConstraint(name) => {
                    self.prepare_validate_constraint(name, sql);
                }
                TableAlterOption::AlterConstraint { name, change } => {
                    self.prepare_alter_constraint(name, *change, sql);
                }
            }
            false
        });
    }

    // [spec:pgorm:req:sql.ddl.drop-rename-truncate+4]
    pub(crate) fn prepare_table_rename_statement(
        &self,
        rename: &TableRenameStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER TABLE ").unwrap();
        self.prepare_table_name(&rename.from_name, sql);
        write!(sql, " RENAME TO ").unwrap();
        rename.to_name.prepare(sql.as_writer());
    }

    /// Translate [`ColumnRenameStatement`] into SQL statement.
    // [spec:pgorm:req:sql.ddl.alter-table+13]
    pub(crate) fn prepare_column_rename_statement(
        &self,
        rename: &ColumnRenameStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER TABLE ").unwrap();
        self.prepare_table_name(&rename.table, sql);
        write!(sql, " RENAME COLUMN ").unwrap();
        rename.from_name.prepare(sql.as_writer());
        write!(sql, " TO ").unwrap();
        rename.to_name.prepare(sql.as_writer());
    }

    /// Translate [`TableCreateStatement`] into SQL statement.
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub(crate) fn prepare_table_create_statement(
        &self,
        create: &TableCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE TABLE ").unwrap();

        self.prepare_create_table_if_not_exists(create, sql);

        self.prepare_table_name(&create.table, sql);

        write!(sql, " ( ").unwrap();
        let mut first = true;

        create.columns.iter().for_each(|column_def| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_column_def(column_def, sql);
            first = false;
        });

        // [spec:pgorm:req:sql.ddl.create-table+16]
        if let Some(key) = &create.primary_key {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_table_key("PRIMARY KEY", key, sql);
            first = false;
        }

        create.unique_keys.iter().for_each(|key| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_table_key("UNIQUE", key, sql);
            first = false;
        });

        create.foreign_keys.iter().for_each(|foreign_key| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_foreign_key_create_statement_internal(foreign_key, sql, Mode::Creation);
            first = false;
        });

        create.check.iter().for_each(|check| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_check_constraint(check, sql);
            first = false;
        });

        write!(sql, " )").unwrap();

        if let Some(raw_suffix) = create.raw_suffix {
            write!(sql, " {raw_suffix}").unwrap();
        }
    }

    /// Translate [`ColumnSpec`] into SQL statement.
    fn prepare_column_spec(&self, column_spec: &ColumnSpec, sql: &mut dyn SqlWriter) {
        match column_spec {
            ColumnSpec::Null => write!(sql, "NULL").unwrap(),
            ColumnSpec::NotNull { name, no_inherit } => {
                self.prepare_not_null(name.as_ref(), *no_inherit, sql)
            }
            ColumnSpec::Default(value) => {
                write!(sql, "DEFAULT ").unwrap();
                self.prepare_default_operand(value, sql);
            }
            // The serial family carries auto-increment in the column's type,
            // rendered by `prepare_column_auto_increment`; there is no
            // trailing keyword to spell here.
            ColumnSpec::AutoIncrement => {}
            ColumnSpec::Check(check) => self.prepare_check_constraint(check, sql),
            ColumnSpec::Generated { expr, kind } => self.prepare_generated_column(expr, *kind, sql),
            // [spec:pgorm:req:sql.ddl.column-def+13]
            ColumnSpec::Identity(generation, options) => {
                write!(sql, "GENERATED {} AS IDENTITY", generation.keyword()).unwrap();
                self.prepare_identity_options(options.as_ref(), sql);
            }
            ColumnSpec::RawSuffix(sql_text) => write!(sql, "{sql_text}").unwrap(),
            ColumnSpec::Comment(_) => {}
        }
    }

    /// Translate [`CommentStatement`] into SQL statement.
    // [spec:pgorm:req:sql.ddl.comment+5]
    pub(crate) fn prepare_comment_statement(
        &self,
        statement: &CommentStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "COMMENT ON ").unwrap();
        match &statement.target {
            CommentTarget::Table(table) => {
                write!(sql, "TABLE ").unwrap();
                self.prepare_table_name(table, sql);
            }
            CommentTarget::Column(table, column) => {
                write!(sql, "COLUMN ").unwrap();
                self.prepare_table_name(table, sql);
                write!(sql, ".").unwrap();
                column.prepare(sql.as_writer());
            }
        }
        write!(sql, " IS ").unwrap();
        self.prepare_comment_text(&statement.comment, sql);
    }

    /// Write comment text as a string literal: single quotes around it, every
    /// embedded single quote doubled — and, when the text holds a backslash,
    /// every backslash doubled and the literal written as an `E'...'`.
    ///
    /// Comment text has no bind form to fall back on — a DDL statement yields
    /// SQL alone — so this quoting is the whole injection boundary for it, and
    /// it must not depend on a server setting. Doubling alone does: under
    /// `standard_conforming_strings = off` a backslash escapes the character
    /// after it, so text ending `\'` would be written `\''`, read as an
    /// escaped quote followed by a *closing* one, and everything after it read
    /// as SQL. The `E'...'` says what the backslashes are under either
    /// setting, which is why it is reached for when, and only when, there is a
    /// backslash to say something about: text without one renders exactly as
    /// it always has.
    ///
    /// The doubling is deliberately not
    /// [`escape_string`](QueryBuilder::escape_string), which maps control
    /// characters to their `E''` spellings as well. Comment text is prose and
    /// a newline in it is a newline; this rewrites what the grammar forces and
    /// nothing else.
    // [spec:pgorm:req:sql.ddl.comment+5]
    fn prepare_comment_text(&self, comment: &str, sql: &mut dyn SqlWriter) {
        let quoted = comment.replace('\'', "''");
        if comment.contains('\\') {
            write!(sql, "E'{}'", quoted.replace('\\', "\\\\")).unwrap();
        } else {
            write!(sql, "'{}'", quoted).unwrap();
        }
    }

    /// Translate [`TableDropStatement`] into SQL statement.
    // [spec:pgorm:req:sql.ddl.drop-rename-truncate+4]
    pub(crate) fn prepare_table_drop_statement(
        &self,
        drop: &TableDropStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "DROP TABLE ").unwrap();

        if drop.if_exists {
            write!(sql, "IF EXISTS ").unwrap();
        }

        drop.tables.iter().fold(true, |first, table| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_table_name(table, sql);
            false
        });

        for drop_opt in drop.options.iter() {
            self.prepare_table_drop_opt(drop_opt, sql);
        }
    }

    /// Translate [`TableDropOpt`] into SQL statement.
    fn prepare_table_drop_opt(&self, drop_opt: &TableDropOpt, sql: &mut dyn SqlWriter) {
        write!(
            sql,
            " {}",
            match drop_opt {
                TableDropOpt::Restrict => "RESTRICT",
                TableDropOpt::Cascade => "CASCADE",
            }
        )
        .unwrap();
    }

    /// Translate [`TableTruncateStatement`] into SQL statement.
    pub(crate) fn prepare_table_truncate_statement(
        &self,
        truncate: &TableTruncateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "TRUNCATE TABLE ").unwrap();
        self.prepare_table_name(&truncate.table, sql);
    }

    /// `[CONSTRAINT "name" ]CHECK (<expr>)[ ENFORCED | NOT ENFORCED]`, the
    /// same at column level, at table level and after `ALTER TABLE`'s `ADD`.
    // [spec:pgorm:req:sql.ddl.enforcement]
    pub(crate) fn prepare_check_constraint(&self, check: &Check, sql: &mut dyn SqlWriter) {
        self.prepare_constraint_name(check.name.as_ref(), sql);
        write!(sql, "CHECK (").unwrap();
        self.prepare_simple_expr(&check.expr, sql);
        write!(sql, ")").unwrap();
        if check.no_inherit {
            write!(sql, " NO INHERIT").unwrap();
        }
        if let Some(enforcement) = check.enforcement {
            write!(sql, "{}", enforcement.clause()).unwrap();
        }
    }

    /// Translate IF NOT EXISTS expression in [`TableCreateStatement`].
    fn prepare_create_table_if_not_exists(
        &self,
        create: &TableCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        if create.if_not_exists {
            write!(sql, "IF NOT EXISTS ").unwrap();
        }
    }

    // INDEX

    /// Write a table key as its constraint: `keyword` is `PRIMARY KEY` or
    /// `UNIQUE`, and `NULLS NOT DISTINCT`, which only a unique key can carry,
    /// goes before the column list, where the standalone `CREATE INDEX` of
    /// `prepare_index_create_statement` puts it after. The key alone has
    /// deferrability to write, and its columns are plain names: it has no
    /// entry that could carry an ordering, an operator class or an expression.
    /// A temporal key's `WITHOUT OVERLAPS` column closes the list.
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub(super) fn prepare_table_key<K>(
        &self,
        keyword: &str,
        key: &TableKey<K>,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(name) = &key.name {
            write!(sql, "CONSTRAINT ").unwrap();
            name.prepare(sql.as_writer());
            write!(sql, " ").unwrap();
        }

        write!(sql, "{keyword}").unwrap();
        if key.nulls_not_distinct {
            write!(sql, " NULLS NOT DISTINCT").unwrap();
        }
        write!(sql, " (").unwrap();
        key.columns.iter().fold(true, |first, name| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            name.prepare(sql.as_writer());
            false
        });
        // [spec:pgorm:req:sql.ddl.create-table+16]
        if let Some(period) = &key.without_overlaps {
            write!(sql, ", ").unwrap();
            period.prepare(sql.as_writer());
            write!(sql, " WITHOUT OVERLAPS").unwrap();
        }
        write!(sql, ")").unwrap();

        if !key.include.is_empty() {
            write!(sql, " INCLUDE (").unwrap();
            key.include.iter().fold(true, |first, name| {
                if !first {
                    write!(sql, ", ").unwrap();
                }
                name.prepare(sql.as_writer());
                false
            });
            write!(sql, ")").unwrap();
        }

        // [spec:pgorm:req:sql.ddl.deferrability+4]
        if let Some(deferrability) = key.deferrability {
            write!(sql, "{}", deferrability.clause()).unwrap();
        }
    }

    // [spec:pgorm:req:sql.ddl.index-create+11]
    pub(crate) fn prepare_index_create_statement(
        &self,
        create: &IndexCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE ").unwrap();
        if create.kind == IndexKind::Unique {
            write!(sql, "UNIQUE ").unwrap();
        }
        write!(sql, "INDEX ").unwrap();

        if create.if_not_exists {
            write!(sql, "IF NOT EXISTS ").unwrap();
        }

        if let Some(name) = &create.index.name {
            name.prepare(sql.as_writer());
        }

        write!(sql, " ON ").unwrap();
        self.prepare_table_name(&create.table, sql);

        self.prepare_index_type(&create.index_type, sql);
        write!(sql, " ").unwrap();
        self.prepare_index_columns(&create.index.columns, sql);

        // [spec:pgorm:req:sql.ddl.index-create+11]
        if !create.include.is_empty() {
            write!(sql, " INCLUDE (").unwrap();
            create.include.iter().fold(true, |first, name| {
                if !first {
                    write!(sql, ", ").unwrap();
                }
                name.prepare(sql.as_writer());
                false
            });
            write!(sql, ")").unwrap();
        }

        if create.nulls_not_distinct && create.kind == IndexKind::Unique {
            write!(sql, " NULLS NOT DISTINCT").unwrap();
        }

        // The predicate closes the statement, after every clause that describes
        // the index itself.
        // [spec:pgorm:req:sql.ddl.index-create+11]
        self.prepare_condition(&create.r#where, "WHERE", sql);
    }

    // [spec:pgorm:req:sql.ddl.index-drop+3]
    pub(crate) fn prepare_index_drop_statement(
        &self,
        drop: &IndexDropStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "DROP INDEX ").unwrap();

        if drop.if_exists {
            write!(sql, "IF EXISTS ").unwrap();
        }

        if let Some(schema) = drop.table.as_ref().and_then(TableName::schema) {
            schema.prepare(sql.as_writer());
            write!(sql, ".").unwrap();
        }
        drop.name.prepare(sql.as_writer());
    }

    fn prepare_index_type(&self, col_index_type: &Option<IndexType>, sql: &mut dyn SqlWriter) {
        if let Some(index_type) = col_index_type {
            write!(
                sql,
                " USING {}",
                match index_type {
                    IndexType::BTree => "BTREE".to_owned(),
                    IndexType::Gin => "GIN".to_owned(),
                    IndexType::Hash => "HASH".to_owned(),
                    // [spec:pgorm:req:sql.render.ident-quoting+7]
                    IndexType::Named(method) => {
                        let mut part = String::new();
                        TypeName::prepare_part(method, Position::AccessMethod, &mut part);
                        part
                    }
                }
            )
            .unwrap();
        }
    }

    /// Write an index's column list: each entry's target, then its operator
    /// class, then its order — PostgreSQL's order for the three, and the reason
    /// an expression entry composes with a direction the way a named one does.
    // [spec:pgorm:req:sql.ddl.index-create+11]
    fn prepare_index_columns(&self, columns: &[IndexColumn], sql: &mut dyn SqlWriter) {
        write!(sql, "(").unwrap();
        columns.iter().fold(true, |first, col| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            match &col.target {
                IndexColumnTarget::Name(name) => name.prepare(sql.as_writer()),
                // The parentheses are the grammar's, not decoration: without
                // them PostgreSQL reads the expression as a column name.
                IndexColumnTarget::Expr(expr) => {
                    write!(sql, "(").unwrap();
                    self.prepare_simple_expr(expr, sql);
                    write!(sql, ")").unwrap();
                }
            }
            if let Some(class) = &col.operator_class {
                write!(sql, " ").unwrap();
                class.prepare(sql.as_writer());
            }
            if let Some(order) = &col.order {
                match order {
                    IndexOrder::Asc => write!(sql, " ASC").unwrap(),
                    IndexOrder::Desc => write!(sql, " DESC").unwrap(),
                }
            }
            false
        });
        write!(sql, ")").unwrap();
    }

    // FOREIGN KEY

    // [spec:pgorm:req:sql.ddl.foreign-key+10]
    fn prepare_foreign_key_create_statement_internal(
        &self,
        create: &ForeignKeyCreateStatement,
        sql: &mut dyn SqlWriter,
        mode: Mode,
    ) {
        if mode == Mode::Alter {
            write!(sql, "ALTER TABLE ").unwrap();
            self.prepare_table_name(&create.foreign_key.table, sql);
            write!(sql, " ").unwrap();
        }

        if mode != Mode::Creation {
            write!(sql, "ADD ").unwrap();
        }

        if let Some(name) = &create.foreign_key.name {
            write!(sql, "CONSTRAINT ").unwrap();
            name.prepare(sql.as_writer());
            write!(sql, " ").unwrap();
        }

        write!(sql, "FOREIGN KEY (").unwrap();
        create.foreign_key.columns().fold(true, |first, (col, _)| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            col.prepare(sql.as_writer());
            false
        });
        // [spec:pgorm:req:sql.ddl.foreign-key+10]
        if let Some((period, _)) = &create.foreign_key.period {
            write!(sql, ", PERIOD ").unwrap();
            period.prepare(sql.as_writer());
        }
        write!(sql, ")").unwrap();

        write!(sql, " REFERENCES ").unwrap();
        self.prepare_table_name(&create.foreign_key.ref_table, sql);
        write!(sql, " ").unwrap();

        write!(sql, "(").unwrap();
        create.foreign_key.columns().fold(true, |first, (_, col)| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            col.prepare(sql.as_writer());
            false
        });
        if let Some((_, ref_period)) = &create.foreign_key.period {
            write!(sql, ", PERIOD ").unwrap();
            ref_period.prepare(sql.as_writer());
        }
        write!(sql, ")").unwrap();

        if let Some(foreign_key_action) = &create.foreign_key.on_delete {
            write!(sql, " ON DELETE ").unwrap();
            self.prepare_foreign_key_action(foreign_key_action, sql);
        }

        if let Some(foreign_key_action) = &create.foreign_key.on_update {
            write!(sql, " ON UPDATE ").unwrap();
            self.prepare_foreign_key_action(foreign_key_action, sql);
        }

        // [spec:pgorm:req:sql.ddl.deferrability+4]
        if let Some(deferrability) = create.foreign_key.deferrability {
            write!(sql, "{}", deferrability.clause()).unwrap();
        }

        // [spec:pgorm:req:sql.ddl.enforcement]
        if let Some(enforcement) = create.foreign_key.enforcement {
            write!(sql, "{}", enforcement.clause()).unwrap();
        }
    }

    /// Translate [`ForeignKeyCreateStatement`] into SQL statement.
    pub(crate) fn prepare_foreign_key_create_statement(
        &self,
        create: &ForeignKeyCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        self.prepare_foreign_key_create_statement_internal(create, sql, Mode::Alter)
    }

    /// Translate [`ForeignKeyAction`] into SQL statement.
    pub(crate) fn prepare_foreign_key_action(
        &self,
        foreign_key_action: &ForeignKeyAction,
        sql: &mut dyn SqlWriter,
    ) {
        write!(
            sql,
            "{}",
            match foreign_key_action {
                ForeignKeyAction::Restrict => "RESTRICT",
                ForeignKeyAction::Cascade => "CASCADE",
                ForeignKeyAction::SetNull => "SET NULL",
                ForeignKeyAction::NoAction => "NO ACTION",
                ForeignKeyAction::SetDefault => "SET DEFAULT",
            }
        )
        .unwrap()
    }

    // ESCAPE

    /// Escape a SQL string literal.
    ///
    /// Only characters PostgreSQL's `E''` escape language actually defines
    /// are mapped; every other byte — 0x1A and the double quote included —
    /// is a legal string character and passes through raw, so it round-trips
    /// instead of losing its backslash server-side.
    // [spec:pgorm:req:sql.render.string-escape+1]
    pub fn escape_string(&self, string: &str) -> String {
        string
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\0', "\\0")
            .replace('\x08', "\\b")
            .replace('\x09', "\\t")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    }

    /// Unescape a SQL string literal
    pub fn unescape_string(&self, string: &str) -> String {
        let mut escape = false;
        let mut output = String::new();
        for c in string.chars() {
            if !escape && c == '\\' {
                escape = true;
            } else if escape {
                write!(
                    output,
                    "{}",
                    match c {
                        '0' => '\0',
                        'b' => '\x08',
                        't' => '\x09',
                        'n' => '\n',
                        'r' => '\r',
                        c => c,
                    }
                )
                .unwrap();
                escape = false;
            } else {
                write!(output, "{c}").unwrap();
            }
        }
        output
    }

    // TABLE NAME
    /// Translate [`TableName`] into SQL statement.
    fn prepare_table_name(&self, name: &TableName, sql: &mut dyn SqlWriter) {
        match name {
            TableName::Table(table) => table.prepare(sql.as_writer()),
            TableName::SchemaTable(schema, table) => {
                schema.prepare(sql.as_writer());
                write!(sql, ".").unwrap();
                table.prepare(sql.as_writer());
            }
        }
    }

    /// Translate [`NamedTable`] into SQL statement.
    // [spec:pgorm:def:sql.types.table-ref+7]
    fn prepare_named_table(&self, table: &NamedTable, sql: &mut dyn SqlWriter) {
        self.prepare_table_name(&table.name, sql);
        if let Some(alias) = &table.alias {
            write!(sql, " AS ").unwrap();
            alias.prepare(sql.as_writer());
        }
    }

    // TYPE BUILDER
    // [spec:pgorm:req:sql.ddl.type-enum+7]
    fn prepare_create_as_type(&self, as_type: &TypeAs, sql: &mut dyn SqlWriter) {
        match as_type {
            TypeAs::Enum(values) => {
                write!(sql, "ENUM (").unwrap();
                for (count, val) in values.iter().enumerate() {
                    if count > 0 {
                        write!(sql, ", ").unwrap();
                    }
                    sql.push_param(val.as_str().into());
                }
                write!(sql, ")").unwrap();
            }
            TypeAs::Composite(attributes) => self.prepare_composite_attributes(attributes, sql),
            TypeAs::Range(range) => self.prepare_range_definition(range, sql),
        }
    }

    fn prepare_drop_type_opt(&self, opt: &TypeDropOpt, sql: &mut dyn SqlWriter) {
        write!(
            sql,
            "{}",
            match opt {
                TypeDropOpt::Cascade => "CASCADE",
                TypeDropOpt::Restrict => "RESTRICT",
            }
        )
        .unwrap()
    }

    // [spec:pgorm:req:sql.render.ddl.enum-type+5] (ALTER TYPE label operands parameterized)
    fn prepare_alter_type_opt(&self, opt: &TypeAlterOpt, sql: &mut dyn SqlWriter) {
        match opt {
            TypeAlterOpt::Add(value, placement) => {
                write!(sql, " ADD VALUE ").unwrap();
                match placement {
                    Some(add_option) => match add_option {
                        TypeAlterAddOpt::Before(before_value) => {
                            sql.push_param(value.as_str().into());
                            write!(sql, " BEFORE ").unwrap();
                            sql.push_param(before_value.as_str().into());
                        }
                        TypeAlterAddOpt::After(after_value) => {
                            sql.push_param(value.as_str().into());
                            write!(sql, " AFTER ").unwrap();
                            sql.push_param(after_value.as_str().into());
                        }
                    },
                    None => sql.push_param(value.as_str().into()),
                }
            }
            TypeAlterOpt::Rename(new_name) => {
                write!(sql, " RENAME TO ").unwrap();
                new_name.prepare(sql.as_writer());
            }
            TypeAlterOpt::RenameValue(existing, new_name) => {
                write!(sql, " RENAME VALUE ").unwrap();
                sql.push_param(existing.as_str().into());
                write!(sql, " TO ").unwrap();
                sql.push_param(new_name.as_str().into());
            }
        }
    }

    // [spec:pgorm:req:sql.ddl.type-enum+7]
    // [spec:pgorm:req:sql.render.ddl.enum-type+5]
    pub(crate) fn prepare_type_create_statement(
        &self,
        create: &TypeCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE TYPE ").unwrap();

        self.prepare_type_ref(&create.name, sql);

        if let Some(as_type) = &create.as_type {
            write!(sql, " AS ").unwrap();
            self.prepare_create_as_type(as_type, sql);
        }
    }

    // [spec:pgorm:req:sql.ddl.type-alter-drop+7]
    pub(crate) fn prepare_type_drop_statement(
        &self,
        drop: &TypeDropStatement,
        sql: &mut dyn SqlWriter,
    ) {
        self.prepare_type_names_drop("TYPE", drop.if_exists, drop.names_iter(), sql);

        if let Some(option) = &drop.option {
            write!(sql, " ").unwrap();
            self.prepare_drop_type_opt(option, sql);
        }
    }

    // [spec:pgorm:req:sql.ddl.type-alter-drop+7]
    pub(crate) fn prepare_type_alter_statement(
        &self,
        alter: &TypeAlterStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER TYPE ").unwrap();
        self.prepare_type_ref(&alter.name, sql);
        self.prepare_alter_type_opt(&alter.option, sql);
    }

    /// Translate [`TypeRef`] into SQL statement.
    fn prepare_type_ref(&self, type_ref: &TypeRef, sql: &mut dyn SqlWriter) {
        match type_ref {
            TypeRef::Type(name) => {
                name.prepare(sql.as_writer());
            }
            TypeRef::SchemaType(schema, name) => {
                schema.prepare(sql.as_writer());
                write!(sql, ".").unwrap();
                name.prepare(sql.as_writer());
            }
            TypeRef::DatabaseSchemaType(database, schema, name) => {
                database.prepare(sql.as_writer());
                write!(sql, ".").unwrap();
                schema.prepare(sql.as_writer());
                write!(sql, ".").unwrap();
                name.prepare(sql.as_writer());
            }
        }
    }

    // EXTENSION
    // [spec:pgorm:req:sql.ddl.extension+5]
    // [spec:pgorm:sem:sql.render.ddl.extension+3] (CREATE EXTENSION)
    pub(crate) fn prepare_extension_create_statement(
        &self,
        create: &ExtensionCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE EXTENSION ").unwrap();

        if create.if_not_exists {
            write!(sql, "IF NOT EXISTS ").unwrap()
        }

        create.name.prepare(sql.as_writer());

        if let Some(schema) = create.schema.as_ref() {
            write!(sql, " WITH SCHEMA ").unwrap();
            schema.prepare(sql.as_writer());
        }

        if let Some(version) = create.version.as_ref() {
            write!(sql, " VERSION ").unwrap();
            let mut literal = String::new();
            self.write_string_quoted(version, &mut literal);
            write!(sql, "{literal}").unwrap();
        }

        if create.cascade {
            write!(sql, " CASCADE").unwrap();
        }
    }

    // [spec:pgorm:req:sql.ddl.extension+5]
    // [spec:pgorm:sem:sql.render.ddl.extension+3] (DROP EXTENSION)
    pub(crate) fn prepare_extension_drop_statement(
        &self,
        drop: &ExtensionDropStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "DROP EXTENSION ").unwrap();

        if drop.if_exists {
            write!(sql, "IF EXISTS ").unwrap();
        }

        drop.name.prepare(sql.as_writer());

        match drop.option {
            Some(ExtensionDropOpt::Cascade) => write!(sql, " CASCADE").unwrap(),
            Some(ExtensionDropOpt::Restrict) => write!(sql, " RESTRICT").unwrap(),
            None => {}
        }
    }
}

impl SubQueryStatement {
    pub(crate) fn prepare_statement(&self, sql: &mut dyn SqlWriter) {
        use SubQueryStatement::*;
        match self {
            SelectStatement(stmt) => QueryBuilder.prepare_select_statement(stmt, sql),
            InsertStatement(stmt) => QueryBuilder.prepare_insert_statement(stmt, sql),
            UpdateStatement(stmt) => QueryBuilder.prepare_update_statement(stmt, sql),
            DeleteStatement(stmt) => QueryBuilder.prepare_delete_statement(stmt, sql),
            MergeStatement(stmt) => QueryBuilder.prepare_merge_statement(stmt, sql),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Creation,
    Alter,
    TableAlter,
}
