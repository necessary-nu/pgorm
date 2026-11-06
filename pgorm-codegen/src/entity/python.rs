//! The Python writer: a schema's declared models as one module of
//! `pgorm.Model` declarations, their relations, junction paths, disjoint
//! models and one-of references, laid out as black lays out Python.

mod layout;

use self::layout::{Py, from_import, string};
use super::declared::{
    DdlType, Declared, DeclaredColumn, DeclaredModel, Fill, Kind, Literal, ModelWriterOptions,
    Naming, Plan, TypeRef, relation_kind,
};
use crate::{
    CaseExpr, ColumnExpr, CurrentKeyword, Entities, EntityTransformer, Error, Operator, OutputFile,
    WriterOutput, parse_schema,
};
use heck::ToShoutySnakeCase;
use pgorm_query::{GeneratedKind, IdentityGeneration};
use std::collections::BTreeSet;

/// The file the Python writer writes.
pub const PYTHON_MODULE: &str = "models.py";

/// Every name the module may import, which no name it declares takes.
const IMPORTABLE: [&str; 16] = [
    "Column",
    "CreatedMultirange",
    "CreatedRange",
    "Decimal",
    "Disjoint",
    "Model",
    "OneOf",
    "TypeName",
    "bind",
    "case_when",
    "col",
    "current_date",
    "current_time",
    "current_timestamp",
    "func",
    "s",
];

/// Python's keywords, which a name the module declares is never.
const KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Generate the Python module of a schema's declared models straight from
/// DDL text: [`parse_schema`], [`EntityTransformer::entities`] and
/// [`PythonWriter::generate`].
// [spec:pgorm:def:codegen.models]
pub fn python_models_from_sql(
    sql: &str,
    options: ModelWriterOptions,
) -> Result<WriterOutput, Error> {
    let entities = EntityTransformer::entities(parse_schema(sql)?, &options.unkinded_disjoint)?;
    Ok(PythonWriter::new(&entities, &options)?.generate())
}

/// A schema's declared models, held to what a Python module can name, ready
/// to be written as one.
#[derive(Debug)]
pub struct PythonWriter {
    source: String,
}

impl PythonWriter {
    /// Read `entities` as declared models and lay out their module, refusing
    /// what a declared model cannot say and a name Python cannot declare.
    // [spec:pgorm:def:codegen.models]
    // [spec:pgorm:req:codegen.models.refused]
    pub fn new(entities: &Entities, options: &ModelWriterOptions) -> Result<Self, Error> {
        let declared = Declared::read(entities, options)?;
        let plan = Plan::new(&declared, &NAMING)?;
        let source = Module::default().write(&plan)?;
        Ok(Self { source })
    }

    /// The module, as [`PYTHON_MODULE`].
    // [spec:pgorm:def:codegen.models]
    pub fn generate(self) -> WriterOutput {
        WriterOutput {
            files: vec![OutputFile {
                name: PYTHON_MODULE.to_owned(),
                content: self.source,
            }],
        }
    }
}

/// How Python names what the module declares.
const NAMING: Naming = Naming {
    language: "Python",
    keywords: &KEYWORDS,
    importable: &IMPORTABLE,
    dollar: false,
    joined: |parts| parts.to_owned(),
    constant: |name| name.to_shouty_snake_case(),
};

/// What the module imports, gathered as it is written.
#[derive(Default)]
struct Module {
    imports: BTreeSet<&'static str>,
    decimal: bool,
    schema: bool,
}

impl Module {
    fn import(&mut self, name: &'static str) -> String {
        self.imports.insert(name);
        name.to_owned()
    }

    // [spec:pgorm:sem:codegen.models.columns]
    // [spec:pgorm:sem:codegen.models.relations]
    // [spec:pgorm:sem:codegen.models.disjoint]
    // [spec:pgorm:sem:codegen.models.one-of]
    fn write(&mut self, plan: &Plan<'_>) -> Result<String, Error> {
        let declared = plan.declared;
        let mut blocks: Vec<Vec<String>> = Vec::new();
        let mut types = Vec::new();
        for ty in declared.types.enums.keys() {
            let call = self.type_call("TypeName", ty, None);
            types.push(assign(&plan.type_name(ty), call));
        }
        for (ty, (subtype, multirange)) in &declared.types.ranges {
            let class = match multirange {
                true => "CreatedMultirange",
                false => "CreatedRange",
            };
            let call = self.type_call(class, ty, Some(subtype));
            types.push(assign(&plan.type_name(ty), call));
        }
        blocks.push(types);
        for (model, name) in declared.models.iter().zip(&plan.names.models) {
            let statement = self.model(plan, model, name)?;
            blocks.push(vec![statement]);
        }
        let mut lines = Vec::new();
        for (relation, name) in plan.relations.iter().zip(&plan.names.relations) {
            let mut items = vec![
                Py::Atom(plan.model(relation.to)),
                Py::keyword("from_", fields(&relation.from_columns)),
                Py::keyword("to", fields(&relation.to_columns)),
            ];
            let actions = [
                ("on_delete", relation.on_delete),
                ("on_update", relation.on_update),
            ];
            for (option, action) in actions {
                if let Some(action) = action {
                    items.push(Py::keyword(option, Py::Atom(string(action))));
                }
            }
            let callee = format!(
                "{}.{}",
                plan.model(relation.from),
                relation_kind(&relation.kind)
            );
            lines.push(assign(name, Py::call(&callee, items)));
        }
        blocks.push(lines);
        let mut lines = Vec::new();
        for (path, name) in plan.related.iter().zip(&plan.names.related) {
            let relation = plan
                .names
                .relations
                .get(path.relation)
                .cloned()
                .unwrap_or_default();
            let via = plan
                .names
                .relations
                .get(path.via)
                .cloned()
                .unwrap_or_default();
            let callee = format!("{}.related", plan.model(path.from));
            let items = vec![Py::Atom(relation), Py::keyword("via", Py::Atom(via))];
            lines.push(assign(name, Py::call(&callee, items)));
        }
        blocks.push(lines);
        let mut lines = Vec::new();
        for (disjoint, name) in plan.disjoints.iter().zip(&plan.names.disjoints) {
            let kinds = disjoint
                .kinds
                .iter()
                .map(|kind| {
                    let model = kind
                        .model
                        .map_or_else(|| "None".to_owned(), |model| plan.model(model));
                    (kind.value.clone(), Py::Atom(model))
                })
                .collect();
            let mut items = vec![
                Py::Atom(string(&disjoint.name)),
                Py::Atom(plan.model(disjoint.base)),
                Py::dict(kinds),
            ];
            match &disjoint.kinded {
                Some((column, enum_name, schema)) => {
                    if column != "kind" {
                        items.push(Py::keyword("kind", Py::Atom(string(column))));
                    }
                    if let Some(enum_name) = enum_name {
                        items.push(Py::keyword("enum_name", Py::Atom(string(enum_name))));
                    }
                    items.push(Py::keyword("schema_name", Py::Atom(string(schema))));
                }
                None => items.push(Py::keyword("unkinded", Py::Atom("True".to_owned()))),
            }
            let callee = self.import("Disjoint");
            lines.push(assign(name, Py::call(&callee, items)));
        }
        for (one_of, name) in plan.one_ofs.iter().zip(&plan.names.one_ofs) {
            let targets = one_of
                .targets
                .iter()
                .map(|target| {
                    let pair = Py::tuple(vec![
                        Py::Atom(string(&target.column)),
                        Py::Atom(plan.model(target.model)),
                    ]);
                    (target.value.clone(), pair)
                })
                .collect();
            let mut items = vec![
                Py::Atom(string(&one_of.name)),
                Py::Atom(plan.model(one_of.model)),
                Py::dict(targets),
            ];
            if one_of.optional {
                items.push(Py::keyword("optional", Py::Atom("True".to_owned())));
            }
            let callee = self.import("OneOf");
            lines.push(assign(name, Py::call(&callee, items)));
        }
        blocks.push(lines);
        blocks.retain(|block| !block.is_empty());
        Ok(self.module(blocks))
    }

    /// `TypeName(name, schema=..)`, or a created range's `CreatedRange(name,
    /// subtype, schema=..)`.
    fn type_call(&mut self, class: &'static str, ty: &TypeRef, subtype: Option<&str>) -> Py {
        let callee = self.import(class);
        let mut items = vec![Py::Atom(string(&ty.name))];
        if let Some(subtype) = subtype {
            items.push(Py::Atom(string(subtype)));
        }
        items.push(Py::keyword("schema", Py::Atom(string(&ty.schema))));
        Py::call(&callee, items)
    }

    /// `Name = Model("table", {..}, ..)`.
    fn model(
        &mut self,
        plan: &Plan<'_>,
        model: &DeclaredModel<'_>,
        name: &str,
    ) -> Result<String, Error> {
        let mut fields = Vec::new();
        for column in &model.columns {
            let field = self.column(plan, column).map_err(|problem| {
                Error::TransformError(format!(
                    "table `{}`: column `{}` {problem}",
                    model.entity.ident(),
                    column.column.name
                ))
            })?;
            fields.push((column.column.name.clone(), field));
        }
        let callee = self.import("Model");
        let mut items = vec![Py::Atom(string(&model.entity.table_name)), Py::dict(fields)];
        let schema = model.entity.schema_name.as_ref().or(plan
            .declared
            .options
            .schema_name
            .as_ref());
        if let Some(schema) = schema {
            items.push(Py::keyword("schema", Py::Atom(string(schema))));
        }
        if !model.unique_keys.is_empty() {
            let keys = model
                .unique_keys
                .iter()
                .map(|key| {
                    let mut entries = vec![("fields".to_owned(), fields_of(&key.columns))];
                    if let Some(name) = &key.name {
                        entries.push(("name".to_owned(), Py::Atom(string(name))));
                    }
                    if key.nulls_not_distinct {
                        entries
                            .push(("nulls_not_distinct".to_owned(), Py::Atom("True".to_owned())));
                    }
                    Py::dict(entries)
                })
                .collect();
            items.push(Py::keyword("unique_keys", Py::list(keys)));
        }
        Ok(assign(name, Py::call(&callee, items)))
    }

    /// `Column(kind, ..)` of one declared column.
    // [spec:pgorm:sem:codegen.models.columns]
    fn column(&mut self, plan: &Plan<'_>, column: &DeclaredColumn<'_>) -> Result<Py, String> {
        let kind = match &column.kind {
            Kind::Scalar(kind) => string(kind),
            Kind::Enum(ty) | Kind::Created(ty) => plan.type_name(ty),
        };
        let mut items = vec![Py::Atom(kind)];
        let yes = || Py::Atom("True".to_owned());
        if column.nullable {
            items.push(Py::keyword("nullable", yes()));
        }
        if column.primary_key {
            items.push(Py::keyword("primary_key", yes()));
        }
        if column.array {
            items.push(Py::keyword("array", yes()));
        }
        if let Fill::Identity(identity) = column.fill {
            let generated = match identity {
                IdentityGeneration::Always => "always",
                IdentityGeneration::ByDefault => "by_default",
            };
            items.push(Py::keyword("generated", Py::Atom(string(generated))));
        }
        if let Some(values) = &column.values {
            let labels = values.iter().map(|label| Py::Atom(string(label))).collect();
            items.push(Py::keyword("values", Py::list(labels)));
        }
        if let Some(ddl) = &column.column_type {
            let ddl = self.ddl(plan, ddl);
            items.push(Py::keyword("column_type", ddl));
        }
        match &column.fill {
            Fill::Value(literal) => {
                let value = self.literal(literal);
                items.push(Py::keyword("default_value", Py::Atom(value)));
            }
            Fill::Bound(literal) => {
                let bind = self.import("bind");
                let value = self.literal(literal);
                items.push(Py::keyword(
                    "default_expr",
                    Py::Atom(format!("{bind}({value})")),
                ));
            }
            Fill::Expr(expr) => {
                let expr = self.expr(expr)?.text;
                items.push(Py::keyword("default_expr", Py::Atom(expr)));
            }
            Fill::Generated(expr, kind) => {
                let option = match kind {
                    GeneratedKind::Stored => "generated_stored",
                    GeneratedKind::Virtual => "generated_virtual",
                };
                let expr = self.expr(expr)?.text;
                items.push(Py::keyword(option, Py::Atom(expr)));
            }
            Fill::AutoIncrement => items.push(Py::keyword("auto_increment", yes())),
            Fill::None | Fill::Identity(_) => {}
        }
        if column.without_overlaps {
            items.push(Py::keyword("without_overlaps", yes()));
        }
        let callee = self.import("Column");
        Ok(Py::call(&callee, items))
    }

    /// A stated column type as `pgorm.schema` spells it.
    fn ddl(&mut self, plan: &Plan<'_>, ddl: &DdlType) -> Py {
        match ddl {
            DdlType::Builtin(name) => Py::Atom(string(name)),
            DdlType::Sized(name, length) => self.data_type(name, &[("length", length.to_string())]),
            DdlType::Numeric(precision, scale) => {
                let mut sizes = vec![("precision", precision.to_string())];
                if *scale != 0 {
                    sizes.push(("scale", scale.to_string()));
                }
                self.data_type("numeric", &sizes)
            }
            DdlType::Interval(fields, precision) => {
                let mut options = Vec::new();
                if let Some(fields) = fields {
                    options.push(("fields", string(fields)));
                }
                if let Some(precision) = precision {
                    options.push(("precision", precision.to_string()));
                }
                self.data_type("interval", &options)
            }
            DdlType::Enum(ty) | DdlType::Created(ty) => Py::Atom(plan.type_name(ty)),
            DdlType::Domain { name, schema, base } => {
                self.schema = true;
                let name = match schema {
                    Some(schema) => {
                        let callee = self.import("TypeName");
                        Py::call(
                            &callee,
                            vec![
                                Py::Atom(string(name)),
                                Py::keyword("schema", Py::Atom(string(schema))),
                            ],
                        )
                    }
                    None => Py::Atom(string(name)),
                };
                let base = self.ddl(plan, base);
                Py::call("s.DataType.domain", vec![name, base])
            }
        }
    }

    fn data_type(&mut self, name: &str, options: &[(&str, String)]) -> Py {
        self.schema = true;
        let mut items = vec![Py::Atom(string(name))];
        for (option, value) in options {
            items.push(Py::keyword(option, Py::Atom(value.clone())));
        }
        Py::call("s.DataType", items)
    }

    /// A literal as the Python value it is.
    fn literal(&mut self, literal: &Literal) -> String {
        match literal {
            Literal::Int(int) => int.to_string(),
            Literal::Float(text) => text.clone(),
            Literal::Decimal(text) => {
                self.decimal = true;
                format!("Decimal({})", string(text))
            }
            Literal::Text(text) => string(text),
            Literal::Bool(true) => "True".to_owned(),
            Literal::Bool(false) => "False".to_owned(),
        }
    }

    /// `expr` as the Python that builds it, an `Expr`.
    // [spec:pgorm:sem:codegen.models.columns]
    fn expr(&mut self, expr: &ColumnExpr) -> Result<Printed, String> {
        Ok(match expr {
            ColumnExpr::Column(name) => {
                let col = self.import("col");
                Printed::primary(format!("{col}({})", string(name)))
            }
            literal @ (ColumnExpr::Int(_)
            | ColumnExpr::BigInt(_)
            | ColumnExpr::Decimal(..)
            | ColumnExpr::Text(_)
            | ColumnExpr::Bool(_)) => {
                let bind = self.import("bind");
                let value = self.operand(literal)?.text;
                Printed::primary(format!("{bind}({value})"))
            }
            ColumnExpr::Null => {
                return Err(
                    "holds NULL in its expression, which a declared model has no word for".into(),
                );
            }
            ColumnExpr::Keyword(keyword) => {
                let function = self.import(match keyword {
                    CurrentKeyword::Date => "current_date",
                    CurrentKeyword::Time => "current_time",
                    CurrentKeyword::Timestamp => "current_timestamp",
                });
                Printed::primary(format!("{function}()"))
            }
            ColumnExpr::Call(name, args) => {
                let func = self.import("func");
                let mut parts = vec![string(name)];
                for arg in args {
                    parts.push(self.operand(arg)?.text);
                }
                Printed::primary(format!("{func}({})", parts.join(", ")))
            }
            ColumnExpr::Cast(operand, cast) => {
                let receiver = self.expr(operand)?.at_least(PRIMARY);
                let type_name = self.import("TypeName");
                let mut parts = vec![string(&cast.name)];
                if let Some(schema) = &cast.schema {
                    parts.push(format!("schema={}", string(schema)));
                }
                let array = if cast.array { ", array=True" } else { "" };
                Printed::primary(format!(
                    "{receiver}.cast({type_name}({}){array})",
                    parts.join(", ")
                ))
            }
            ColumnExpr::Binary(left, op, right) => self.binary(left, *op, right)?,
            ColumnExpr::Not(operand) => {
                let operand = self.expr(operand)?.at_least(UNARY);
                Printed {
                    text: format!("~{operand}"),
                    level: UNARY,
                }
            }
            ColumnExpr::IsNull(operand, negated) => {
                let receiver = self.expr(operand)?.at_least(PRIMARY);
                let method = if *negated { "is_not_null" } else { "is_null" };
                Printed::primary(format!("{receiver}.{method}()"))
            }
            ColumnExpr::Case(case) => self.case(case)?,
        })
    }

    /// `expr` where an operand is taken, a literal written as the value it
    /// is, which the operator binds.
    fn operand(&mut self, expr: &ColumnExpr) -> Result<Printed, String> {
        match super::declared::literal(expr) {
            Some(literal) => Ok(Printed::primary(self.literal(&literal))),
            None => self.expr(expr),
        }
    }

    fn binary(
        &mut self,
        left: &ColumnExpr,
        op: Operator,
        right: &ColumnExpr,
    ) -> Result<Printed, String> {
        let (symbol, level) = match op {
            Operator::Add => ("+", ADD),
            Operator::Sub => ("-", ADD),
            Operator::Mul => ("*", MUL),
            Operator::Div => ("/", MUL),
            Operator::Mod => ("%", MUL),
            Operator::Eq => ("==", COMPARE),
            Operator::Ne => ("!=", COMPARE),
            Operator::Lt => ("<", COMPARE),
            Operator::Gt => (">", COMPARE),
            Operator::Le => ("<=", COMPARE),
            Operator::Ge => (">=", COMPARE),
            Operator::And => ("&", AND),
            Operator::Or => ("|", OR),
            Operator::Concat => {
                let receiver = self.expr(left)?.at_least(PRIMARY);
                let other = self.operand(right)?.text;
                return Ok(Printed::primary(format!("{receiver}.concat({other})")));
            }
        };
        // Comparisons chain in Python, so neither side of one is another.
        let left_level = if level == COMPARE { level + 1 } else { level };
        let left = self.expr(left)?.at_least(left_level);
        let right = self.operand(right)?.at_least(level + 1);
        Ok(Printed {
            text: format!("{left} {symbol} {right}"),
            level,
        })
    }

    fn case(&mut self, case: &CaseExpr) -> Result<Printed, String> {
        let case_when = self.import("case_when");
        let (condition, result) = &case.first;
        let mut text = format!(
            "{case_when}({}, {})",
            self.expr(condition)?.text,
            self.operand(result)?.text
        );
        for (condition, result) in &case.rest {
            text.push_str(&format!(
                ".when({}, {})",
                self.expr(condition)?.text,
                self.operand(result)?.text
            ));
        }
        if let Some(otherwise) = &case.otherwise {
            text.push_str(&format!(".else_({})", self.operand(otherwise)?.text));
        }
        Ok(Printed::primary(text))
    }

    /// The module: its docstring, its imports, then `blocks`, a blank line
    /// between each and between each model.
    // [spec:pgorm:req:codegen.models.layout]
    fn module(&self, blocks: Vec<Vec<String>>) -> String {
        let mut out = format!(
            "\"\"\"Declared models of a schema's tables, @generated by pgorm-codegen {}.\"\"\"\n",
            env!("CARGO_PKG_VERSION")
        );
        if self.decimal {
            out.push_str("\nfrom decimal import Decimal\n");
        }
        if !self.imports.is_empty() || self.schema {
            out.push('\n');
        }
        if !self.imports.is_empty() {
            let names: Vec<&str> = self.imports.iter().copied().collect();
            out.push_str(&from_import("pgorm", &names));
            out.push('\n');
        }
        if self.schema {
            out.push_str("from pgorm import schema as s\n");
        }
        for block in blocks {
            out.push('\n');
            out.push_str(&block.join("\n"));
            out.push('\n');
        }
        out
    }
}

/// A side of a relation's key: one field, or several in order.
fn fields(columns: &[String]) -> Py {
    match columns {
        [one] => Py::Atom(string(one)),
        many => Py::list(many.iter().map(|column| Py::Atom(string(column))).collect()),
    }
}

/// A unique key's fields: one field, or a tuple of several in key order.
fn fields_of(columns: &[String]) -> Py {
    match columns {
        [one] => Py::Atom(string(one)),
        many => Py::tuple(many.iter().map(|column| Py::Atom(string(column))).collect()),
    }
}

fn assign(name: &str, value: Py) -> String {
    Py::Prefixed(format!("{name} = "), Box::new(value)).render(0, 0, 0)
}

const COMPARE: u8 = 1;
const OR: u8 = 2;
const AND: u8 = 3;
const ADD: u8 = 4;
const MUL: u8 = 5;
const UNARY: u8 = 6;
const PRIMARY: u8 = 7;

/// An expression's Python and how tightly it binds, by Python's precedence:
/// comparisons, `|`, `&`, `+`, `*`, `~`, then calls and names.
struct Printed {
    text: String,
    level: u8,
}

impl Printed {
    fn primary(text: String) -> Self {
        Self {
            text,
            level: PRIMARY,
        }
    }

    /// The text, parenthesized where it binds less tightly than `level`.
    fn at_least(self, level: u8) -> String {
        if self.level >= level {
            self.text
        } else {
            format!("({})", self.text)
        }
    }
}
