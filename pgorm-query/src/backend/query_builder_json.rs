//! SQL/JSON: the query functions, the constructors and `IS JSON`.
//!
//! Every form renders as an atom — a call's parentheses, or for `IS JSON`
//! parentheses of its own, as `CASE` takes — so `sql.render.precedence` never
//! has to place one.

use super::*;
use crate::json::{JsonPathTarget, JsonShaping};

impl QueryBuilder {
    /// Translate a [`SqlJson`] into SQL.
    // [spec:pgorm:req:sql.render.sql-json+2]
    pub(super) fn prepare_sql_json(&self, json: &SqlJson, sql: &mut dyn SqlWriter) {
        match json {
            SqlJson::Exists(exists) => {
                write!(sql, "JSON_EXISTS(").unwrap();
                self.prepare_json_path_target(&exists.target, sql);
                if let Some(behavior) = exists.on_error {
                    write!(sql, " {} ON ERROR", exists_behavior(behavior)).unwrap();
                }
                write!(sql, ")").unwrap();
            }
            SqlJson::Value(value) => {
                write!(sql, "JSON_VALUE(").unwrap();
                self.prepare_json_path_target(&value.target, sql);
                self.prepare_json_returning(value.returning.as_ref(), sql);
                for (behavior, on) in [(&value.on_empty, "EMPTY"), (&value.on_error, "ERROR")] {
                    if let Some(behavior) = behavior {
                        write!(sql, " ").unwrap();
                        self.prepare_json_value_behavior(behavior, sql);
                        write!(sql, " ON {on}").unwrap();
                    }
                }
                write!(sql, ")").unwrap();
            }
            SqlJson::Query(query) => self.prepare_json_query(query, sql),
            SqlJson::Object(object) => self.prepare_json_object(object, sql),
            SqlJson::Array(array) => {
                write!(sql, "JSON_ARRAY(").unwrap();
                let mut clauses = Clauses::default();
                for element in &array.elements {
                    clauses.next(", ", sql);
                    self.prepare_json_input(element, sql);
                }
                if array.null_on_null {
                    clauses.next(" ", sql);
                    write!(sql, "NULL ON NULL").unwrap();
                }
                self.prepare_json_returning_clause(array.returning.as_ref(), &mut clauses, sql);
                write!(sql, ")").unwrap();
            }
            SqlJson::ArrayQuery(array) => {
                write!(sql, "JSON_ARRAY(").unwrap();
                self.prepare_select_statement(&array.query, sql);
                self.prepare_json_returning(array.returning.as_ref(), sql);
                write!(sql, ")").unwrap();
            }
            SqlJson::ObjectAgg(agg) => self.prepare_json_objectagg(agg, sql),
            SqlJson::ArrayAgg(agg) => self.prepare_json_arrayagg(agg, sql),
            SqlJson::Parse(parse) => {
                write!(sql, "JSON(").unwrap();
                self.prepare_json_input(&parse.input, sql);
                if parse.unique_keys {
                    write!(sql, " WITH UNIQUE KEYS").unwrap();
                }
                write!(sql, ")").unwrap();
            }
            SqlJson::Scalar(expr) => {
                write!(sql, "JSON_SCALAR(").unwrap();
                self.prepare_json_operand(expr, sql);
                write!(sql, ")").unwrap();
            }
            // The operand is read through `JSON(..)`, because PostgreSQL 19.0
            // serializes a `jsonb` operand as bytes of its binary header —
            // `\x01` for a one-member object. `JSON(..)` turns every operand
            // `JSON_SERIALIZE` takes — `json`, `jsonb`, a string, `bytea`,
            // `FORMAT JSON` or not — into the `json` value `JSON_SERIALIZE`
            // itself makes of all but `jsonb`, and that one it serializes
            // correctly.
            SqlJson::Serialize(serialize) => {
                write!(sql, "JSON_SERIALIZE(JSON(").unwrap();
                self.prepare_json_input(&serialize.input, sql);
                write!(sql, ")").unwrap();
                self.prepare_json_returning(serialize.returning.as_ref(), sql);
                write!(sql, ")").unwrap();
            }
            SqlJson::Is {
                operand,
                test,
                negated,
            } => self.prepare_is_json(operand, test, *negated, sql),
        }
    }

    /// `JSON_QUERY(..)`, whose wrapper or quotes behaviour sits between its
    /// `RETURNING` and its `ON EMPTY`.
    fn prepare_json_query(&self, query: &JsonQuery, sql: &mut dyn SqlWriter) {
        write!(sql, "JSON_QUERY(").unwrap();
        self.prepare_json_path_target(&query.target, sql);
        self.prepare_json_returning(query.returning.as_ref(), sql);
        self.prepare_json_shaping(query.shaping, sql);
        for (behavior, on) in [(&query.on_empty, "EMPTY"), (&query.on_error, "ERROR")] {
            if let Some(behavior) = behavior {
                write!(sql, " ").unwrap();
                self.prepare_json_query_behavior(behavior, sql);
                write!(sql, " ON {on}").unwrap();
            }
        }
        write!(sql, ")").unwrap();
    }

    /// `JSON_QUERY`'s one wrapper-or-quotes clause, when it has one.
    pub(super) fn prepare_json_shaping(
        &self,
        shaping: Option<JsonShaping>,
        sql: &mut dyn SqlWriter,
    ) {
        match shaping {
            Some(JsonShaping::Wrapped) => write!(sql, " WITH UNCONDITIONAL WRAPPER").unwrap(),
            Some(JsonShaping::ConditionallyWrapped) => {
                write!(sql, " WITH CONDITIONAL WRAPPER").unwrap()
            }
            Some(JsonShaping::Unquoted) => write!(sql, " OMIT QUOTES").unwrap(),
            None => {}
        }
    }

    pub(super) fn prepare_json_query_behavior(
        &self,
        behavior: &JsonQueryBehavior,
        sql: &mut dyn SqlWriter,
    ) {
        match behavior {
            JsonQueryBehavior::Null => write!(sql, "NULL").unwrap(),
            JsonQueryBehavior::Error => write!(sql, "ERROR").unwrap(),
            JsonQueryBehavior::EmptyArray => write!(sql, "EMPTY ARRAY").unwrap(),
            JsonQueryBehavior::EmptyObject => write!(sql, "EMPTY OBJECT").unwrap(),
            JsonQueryBehavior::Default(value) => self.prepare_json_default(value, sql),
        }
    }

    /// `JSON_OBJECT(..)`. Each member is written `key : value` rather than
    /// `key VALUE value`: the `VALUE` form takes only a narrow expression as
    /// its key, which a typed placeholder `$1::text` is not.
    fn prepare_json_object(&self, object: &JsonObject, sql: &mut dyn SqlWriter) {
        write!(sql, "JSON_OBJECT(").unwrap();
        let mut clauses = Clauses::default();
        for (key, value) in &object.entries {
            clauses.next(", ", sql);
            self.prepare_json_operand(key, sql);
            write!(sql, " : ").unwrap();
            self.prepare_json_input(value, sql);
        }
        if object.absent_on_null {
            clauses.next(" ", sql);
            write!(sql, "ABSENT ON NULL").unwrap();
        }
        if object.unique_keys {
            clauses.next(" ", sql);
            write!(sql, "WITH UNIQUE KEYS").unwrap();
        }
        self.prepare_json_returning_clause(object.returning.as_ref(), &mut clauses, sql);
        write!(sql, ")").unwrap();
    }

    fn prepare_json_objectagg(&self, agg: &JsonObjectAgg, sql: &mut dyn SqlWriter) {
        write!(sql, "JSON_OBJECTAGG(").unwrap();
        self.prepare_json_operand(&agg.key, sql);
        write!(sql, " : ").unwrap();
        self.prepare_json_input(&agg.value, sql);
        if agg.absent_on_null {
            write!(sql, " ABSENT ON NULL").unwrap();
        }
        if agg.unique_keys {
            write!(sql, " WITH UNIQUE KEYS").unwrap();
        }
        self.prepare_json_returning(agg.returning.as_ref(), sql);
        write!(sql, ")").unwrap();
        self.prepare_json_filter(agg.filter.as_ref(), sql);
    }

    /// `JSON_ARRAYAGG(..)`, whose `FORMAT JSON` follows the value and precedes
    /// the `ORDER BY` — the other way round is a syntax error.
    fn prepare_json_arrayagg(&self, agg: &JsonArrayAgg, sql: &mut dyn SqlWriter) {
        write!(sql, "JSON_ARRAYAGG(").unwrap();
        self.prepare_json_input(&agg.value, sql);
        if !agg.order_by.is_empty() {
            write!(sql, " ORDER BY ").unwrap();
            for (i, order) in agg.order_by.iter().enumerate() {
                if i != 0 {
                    write!(sql, ", ").unwrap();
                }
                self.prepare_order_expr(order, sql);
            }
        }
        if agg.null_on_null {
            write!(sql, " NULL ON NULL").unwrap();
        }
        self.prepare_json_returning(agg.returning.as_ref(), sql);
        write!(sql, ")").unwrap();
        self.prepare_json_filter(agg.filter.as_ref(), sql);
    }

    /// `(operand IS [NOT] JSON[ kind][ WITH UNIQUE KEYS])`, parenthesised as
    /// a whole so it is an atom, and around the operand when that is an
    /// operator expression: `NOT`, `AND` and `OR` bind looser than `IS`.
    fn prepare_is_json(
        &self,
        operand: &SimpleExpr,
        test: &JsonTest,
        negated: bool,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "(").unwrap();
        let wrap = matches!(
            operand,
            SimpleExpr::Unary(..)
                | SimpleExpr::Binary(..)
                | SimpleExpr::Raw(_)
                | SimpleExpr::Template(_)
        );
        if wrap {
            write!(sql, "(").unwrap();
        }
        self.prepare_json_operand(operand, sql);
        if wrap {
            write!(sql, ")").unwrap();
        }
        write!(sql, " IS {}JSON", if negated { "NOT " } else { "" }).unwrap();
        match test.kind {
            JsonKind::Value => {}
            JsonKind::Scalar => write!(sql, " SCALAR").unwrap(),
            JsonKind::Array => write!(sql, " ARRAY").unwrap(),
            JsonKind::Object => write!(sql, " OBJECT").unwrap(),
        }
        if test.unique_keys {
            write!(sql, " WITH UNIQUE KEYS").unwrap();
        }
        write!(sql, ")").unwrap();
    }

    /// The context item, the path and the `PASSING` list. The path is a
    /// value, bound as `text` and cast to `jsonpath`, so its placeholder has
    /// the type the driver writes rather than one the server infers.
    fn prepare_json_path_target(&self, target: &JsonPathTarget, sql: &mut dyn SqlWriter) {
        self.prepare_json_input(&target.context, sql);
        write!(sql, ", CAST(").unwrap();
        sql.push_param_source_typed(Value::from(target.path.as_str()));
        write!(sql, " AS jsonpath)").unwrap();
        self.prepare_json_passing(&target.passing, sql);
    }

    /// ` PASSING value AS "name", ..`, when the list has a variable.
    pub(super) fn prepare_json_passing(
        &self,
        passing: &[(JsonInput, Name)],
        sql: &mut dyn SqlWriter,
    ) {
        for (i, (value, name)) in passing.iter().enumerate() {
            write!(sql, "{}", if i == 0 { " PASSING " } else { ", " }).unwrap();
            self.prepare_json_input(value, sql);
            write!(sql, " AS ").unwrap();
            name.prepare(sql.as_writer());
        }
    }

    /// An expression SQL/JSON reads as JSON, and its `FORMAT JSON`.
    pub(super) fn prepare_json_input(&self, input: &JsonInput, sql: &mut dyn SqlWriter) {
        self.prepare_json_operand(&input.expr, sql);
        if input.format_json {
            write!(sql, " FORMAT JSON").unwrap();
        }
    }

    /// An operand in a position that gives a value no type: a value carries
    /// its own, written after it in both render paths, so a bound statement
    /// and an inlined one build the same JSON. A JSON value is `jsonb` here,
    /// the type the SQL/JSON functions speak.
    // [spec:pgorm:req:sql.render.sql-json+2]
    fn prepare_json_operand(&self, expr: &SimpleExpr, sql: &mut dyn SqlWriter) {
        let SimpleExpr::Value(value) = expr else {
            return self.prepare_simple_expr(expr, sql);
        };
        let type_name = match value {
            Value::Json(_) => Some(std::borrow::Cow::Borrowed("jsonb")),
            value => value.source_type_name(),
        };
        sql.push_param(value.clone());
        if let Some(type_name) = type_name {
            write!(sql, "::{type_name}").unwrap();
        }
    }

    /// A `DEFAULT` behaviour's value, written inline in both render paths:
    /// PostgreSQL refuses a parameter there (`42804`).
    // [spec:pgorm:req:sql.render.sql-json+2]
    fn prepare_json_default(&self, value: &Value, sql: &mut dyn SqlWriter) {
        write!(sql, "DEFAULT {}", self.value_to_string(value)).unwrap();
    }

    pub(super) fn prepare_json_value_behavior(
        &self,
        behavior: &JsonValueBehavior,
        sql: &mut dyn SqlWriter,
    ) {
        match behavior {
            JsonValueBehavior::Null => write!(sql, "NULL").unwrap(),
            JsonValueBehavior::Error => write!(sql, "ERROR").unwrap(),
            JsonValueBehavior::Default(value) => self.prepare_json_default(value, sql),
        }
    }

    fn prepare_json_returning(&self, returning: Option<&ColumnType>, sql: &mut dyn SqlWriter) {
        if let Some(column_type) = returning {
            write!(sql, " RETURNING ").unwrap();
            self.prepare_column_type(column_type, sql);
        }
    }

    /// `RETURNING` as the last of a constructor's clauses, which may also be
    /// the first thing between its parentheses.
    fn prepare_json_returning_clause(
        &self,
        returning: Option<&ColumnType>,
        clauses: &mut Clauses,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(column_type) = returning {
            clauses.next(" ", sql);
            write!(sql, "RETURNING ").unwrap();
            self.prepare_column_type(column_type, sql);
        }
    }

    fn prepare_json_filter(&self, filter: Option<&Condition>, sql: &mut dyn SqlWriter) {
        if let Some(condition) = filter {
            write!(sql, " FILTER (WHERE ").unwrap();
            self.prepare_condition_where(condition, sql);
            write!(sql, ")").unwrap();
        }
    }
}

pub(super) fn exists_behavior(behavior: JsonExistsBehavior) -> &'static str {
    match behavior {
        JsonExistsBehavior::True => "TRUE",
        JsonExistsBehavior::False => "FALSE",
        JsonExistsBehavior::Unknown => "UNKNOWN",
        JsonExistsBehavior::Error => "ERROR",
    }
}

/// The separator before each clause of a constructor whose parentheses may
/// hold nothing before it: none for the first, `sep` after.
#[derive(Debug, Default)]
struct Clauses {
    started: bool,
}

impl Clauses {
    fn next(&mut self, sep: &str, sql: &mut dyn SqlWriter) {
        if self.started {
            write!(sql, "{sep}").unwrap();
        }
        self.started = true;
    }
}
