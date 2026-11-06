//! The TypeScript writer: a schema's declared models as one module of
//! `model(..)` declarations, their relations, junction paths, disjoint
//! models and one-of references, laid out as deno fmt and prettier lay out
//! TypeScript.

mod layout;

use self::layout::{Ts, import, string};
use super::declared::{
    DdlType, Declared, DeclaredColumn, DeclaredModel, Fill, Kind, Literal, ModelWriterOptions,
    Naming, Plan, TypeRef,
};
use crate::{
    CaseExpr, ColumnExpr, CurrentKeyword, Entities, EntityTransformer, Error, Operator, OutputFile,
    RelationType, WriterOutput, parse_schema,
};
use heck::{ToLowerCamelCase, ToSnakeCase};
use pgorm_query::{GeneratedKind, IdentityGeneration};
use std::collections::{BTreeMap, BTreeSet};

/// The file the TypeScript writer writes.
pub const TYPESCRIPT_MODULE: &str = "models.ts";

/// The module the declarations import pgorm's from.
const PGORM: &str = "@necessary/pgorm";

/// Every name the module may import, which no name it declares takes.
const IMPORTABLE: [&str; 16] = [
    "bind",
    "caseWhen",
    "col",
    "column",
    "CreatedMultirange",
    "CreatedRange",
    "currentDate",
    "currentTime",
    "currentTimestamp",
    "DataType",
    "Decimal",
    "disjoint",
    "func",
    "model",
    "oneOf",
    "TypeName",
];

/// JavaScript's reserved words, which a name the module declares is never.
const KEYWORDS: [&str; 46] = [
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// How TypeScript names what the module declares.
const NAMING: Naming = Naming {
    language: "JavaScript",
    keywords: &KEYWORDS,
    importable: &IMPORTABLE,
    dollar: true,
    joined: |parts| parts.to_lower_camel_case(),
    constant: |name| name.to_lower_camel_case(),
};

/// Generate the TypeScript module of a schema's declared models straight
/// from DDL text: [`parse_schema`], [`EntityTransformer::entities`] and
/// [`TypeScriptWriter::generate`].
// [spec:pgorm:def:codegen.models]
pub fn typescript_models_from_sql(
    sql: &str,
    options: ModelWriterOptions,
) -> Result<WriterOutput, Error> {
    let entities = EntityTransformer::entities(parse_schema(sql)?, &options.unkinded_disjoint)?;
    Ok(TypeScriptWriter::new(&entities, &options)?.generate())
}

/// A schema's declared models, held to what a TypeScript module can name,
/// ready to be written as one.
#[derive(Debug)]
pub struct TypeScriptWriter {
    source: String,
}

impl TypeScriptWriter {
    /// Read `entities` as declared models and lay out their module, refusing
    /// what a declared model cannot say and a name JavaScript cannot declare.
    // [spec:pgorm:def:codegen.models]
    // [spec:pgorm:req:codegen.models.refused]
    pub fn new(entities: &Entities, options: &ModelWriterOptions) -> Result<Self, Error> {
        let declared = Declared::read(entities, options)?;
        let plan = Plan::new(&declared, &NAMING)?;
        let fields = plan
            .declared
            .models
            .iter()
            .map(Fields::of)
            .collect::<Result<Vec<_>, _>>()?;
        let source = Module::default().write(&plan, &fields)?;
        Ok(Self { source })
    }

    /// The module, as [`TYPESCRIPT_MODULE`].
    // [spec:pgorm:def:codegen.models]
    pub fn generate(self) -> WriterOutput {
        WriterOutput {
            files: vec![OutputFile {
                name: TYPESCRIPT_MODULE.to_owned(),
                content: self.source,
            }],
        }
    }
}

/// A model's fields: each column's, its name in lowerCamelCase, and
/// whether the model names its columns `renameAll: "snake_case"`, which it
/// does where a field relies on it.
struct Fields {
    by_column: BTreeMap<String, String>,
    snake_case: bool,
}

impl Fields {
    // [spec:pgorm:sem:codegen.models.names]
    fn of(model: &DeclaredModel<'_>) -> Result<Self, Error> {
        let mut by_column = BTreeMap::new();
        let mut taken: BTreeMap<String, String> = BTreeMap::new();
        for column in &model.entity.columns {
            let field = column.name.to_lower_camel_case();
            let table = model.entity.ident();
            if field.is_empty() {
                return Err(Error::TransformError(format!(
                    "table `{table}`: column `{}` names no field in lowerCamelCase",
                    column.name
                )));
            }
            if let Some(first) = taken.insert(field.clone(), column.name.clone()) {
                return Err(Error::TransformError(format!(
                    "table `{table}`: columns `{first}` and `{}` both name the field `{field}`",
                    column.name
                )));
            }
            by_column.insert(column.name.clone(), field);
        }
        let snake_case = by_column
            .iter()
            .any(|(column, field)| field != column && field.to_snake_case() == *column);
        Ok(Self {
            by_column,
            snake_case,
        })
    }

    fn field(&self, column: &str) -> String {
        self.by_column.get(column).cloned().unwrap_or_default()
    }

    /// Whether `column`'s field names it without saying so.
    fn names(&self, column: &str) -> bool {
        let field = self.field(column);
        let named = if self.snake_case {
            field.to_snake_case()
        } else {
            field
        };
        named == column
    }

    /// A side of a relation's key: one field, or several in order.
    fn key(&self, columns: &[String]) -> Ts {
        match columns {
            [one] => Ts::Atom(string(&self.field(one))),
            many => Ts::Array(
                many.iter()
                    .map(|column| Ts::Atom(string(&self.field(column))))
                    .collect(),
            ),
        }
    }
}

/// What the module imports, gathered as it is written.
#[derive(Default)]
struct Module {
    imports: BTreeSet<&'static str>,
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
    fn write(&mut self, plan: &Plan<'_>, fields: &[Fields]) -> Result<String, Error> {
        let declared = plan.declared;
        let fields_of = |model: &DeclaredModel<'_>| {
            declared
                .models
                .iter()
                .position(|candidate| std::ptr::eq(candidate, model))
                .and_then(|at| fields.get(at))
        };
        let mut blocks: Vec<Vec<String>> = Vec::new();
        let mut types = Vec::new();
        for ty in declared.types.enums.keys() {
            let class = self.import("TypeName");
            let value = Ts::Call(
                format!("new {class}"),
                vec![Ts::Atom(string(&ty.name)), schema_option(ty)],
            );
            types.push(export(&plan.type_name(ty), value));
        }
        for (ty, (subtype, multirange)) in &declared.types.ranges {
            let class = self.import(match multirange {
                true => "CreatedMultirange",
                false => "CreatedRange",
            });
            let value = Ts::Call(
                format!("new {class}"),
                vec![
                    Ts::Atom(string(&ty.name)),
                    Ts::Atom(string(subtype)),
                    schema_option(ty),
                ],
            );
            types.push(export(&plan.type_name(ty), value));
        }
        blocks.push(types);
        for ((model, name), fields) in declared.models.iter().zip(&plan.names.models).zip(fields) {
            let statement = self.model(plan, model, name, fields)?;
            blocks.push(vec![statement]);
        }
        let mut lines = Vec::new();
        for (relation, name) in plan.relations.iter().zip(&plan.names.relations) {
            let (Some(from), Some(to)) = (fields_of(relation.from), fields_of(relation.to)) else {
                continue;
            };
            let mut keys = vec![
                ("from".to_owned(), from.key(&relation.from_columns)),
                ("to".to_owned(), to.key(&relation.to_columns)),
            ];
            let actions = [
                ("onDelete", relation.on_delete),
                ("onUpdate", relation.on_update),
            ];
            for (option, action) in actions {
                if let Some(action) = action {
                    keys.push((
                        option.to_owned(),
                        Ts::Atom(string(&action.to_lower_camel_case())),
                    ));
                }
            }
            let method = match relation.kind {
                RelationType::BelongsTo => "belongsTo",
                RelationType::HasOne => "hasOne",
                RelationType::HasMany => "hasMany",
            };
            let callee = format!("{}.{method}", plan.model(relation.from));
            let value = Ts::Call(
                callee,
                vec![Ts::Atom(plan.model(relation.to)), Ts::object(keys)],
            );
            lines.push(export(name, value));
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
            let value = Ts::Call(
                callee,
                vec![
                    Ts::Atom(relation),
                    Ts::object(vec![("via".to_owned(), Ts::Atom(via))]),
                ],
            );
            lines.push(export(name, value));
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
                        .map_or_else(|| "null".to_owned(), |model| plan.model(model));
                    (kind.value.clone(), Ts::Atom(model))
                })
                .collect();
            let mut options = Vec::new();
            match &disjoint.kinded {
                Some((column, enum_name, schema)) => {
                    let field = fields_of(disjoint.base).map(|fields| fields.field(column));
                    if let Some(field) = field.filter(|field| field != "kind") {
                        options.push(("kind".to_owned(), Ts::Atom(string(&field))));
                    }
                    if let Some(enum_name) = enum_name {
                        options.push(("enumName".to_owned(), Ts::Atom(string(enum_name))));
                    }
                    options.push(("schemaName".to_owned(), Ts::Atom(string(schema))));
                }
                None => options.push(("unkinded".to_owned(), Ts::Atom("true".to_owned()))),
            }
            let callee = self.import("disjoint");
            let value = Ts::Call(
                callee,
                vec![
                    Ts::Atom(string(&disjoint.name)),
                    Ts::Atom(plan.model(disjoint.base)),
                    Ts::object(kinds),
                    Ts::object(options),
                ],
            );
            lines.push(export(name, value));
        }
        for (one_of, name) in plan.one_ofs.iter().zip(&plan.names.one_ofs) {
            let Some(own) = fields_of(one_of.model) else {
                continue;
            };
            let targets = one_of
                .targets
                .iter()
                .map(|target| {
                    let pair = Ts::Array(vec![
                        Ts::Atom(string(&own.field(&target.column))),
                        Ts::Atom(plan.model(target.model)),
                    ]);
                    (target.value.clone(), pair)
                })
                .collect();
            let mut args = vec![
                Ts::Atom(string(&one_of.name)),
                Ts::Atom(plan.model(one_of.model)),
                Ts::object(targets),
            ];
            if one_of.optional {
                args.push(Ts::object(vec![(
                    "optional".to_owned(),
                    Ts::Atom("true".to_owned()),
                )]));
            }
            let callee = self.import("oneOf");
            lines.push(export(name, Ts::Call(callee, args)));
        }
        blocks.push(lines);
        blocks.retain(|block| !block.is_empty());
        Ok(self.module(blocks))
    }

    /// `export const Name = model("table", { .. });`.
    fn model(
        &mut self,
        plan: &Plan<'_>,
        model: &DeclaredModel<'_>,
        name: &str,
        fields: &Fields,
    ) -> Result<String, Error> {
        let mut columns = Vec::new();
        for column in &model.columns {
            let declared = self.column(plan, column, fields).map_err(|problem| {
                Error::TransformError(format!(
                    "table `{}`: column `{}` {problem}",
                    model.entity.ident(),
                    column.column.name
                ))
            })?;
            columns.push((fields.field(&column.column.name), declared));
        }
        let mut declaration = Vec::new();
        let schema = model.entity.schema_name.as_ref().or(plan
            .declared
            .options
            .schema_name
            .as_ref());
        if let Some(schema) = schema {
            declaration.push(("schema".to_owned(), Ts::Atom(string(schema))));
        }
        if fields.snake_case {
            declaration.push(("renameAll".to_owned(), Ts::Atom(string("snake_case"))));
        }
        declaration.push(("columns".to_owned(), Ts::object(columns)));
        if !model.unique_keys.is_empty() {
            let keys = model
                .unique_keys
                .iter()
                .map(|key| {
                    let mut entries = vec![("fields".to_owned(), fields.key(&key.columns))];
                    if let Some(name) = &key.name {
                        entries.push(("name".to_owned(), Ts::Atom(string(name))));
                    }
                    if key.nulls_not_distinct {
                        entries.push(("nullsNotDistinct".to_owned(), Ts::Atom("true".to_owned())));
                    }
                    Ts::object(entries)
                })
                .collect();
            declaration.push(("uniqueKeys".to_owned(), Ts::Array(keys)));
        }
        let callee = self.import("model");
        let value = Ts::Call(
            callee,
            vec![
                Ts::Atom(string(&model.entity.table_name)),
                Ts::object(declaration),
            ],
        );
        Ok(export(name, value))
    }

    /// `column(kind, { .. })` of one declared column.
    // [spec:pgorm:sem:codegen.models.columns]
    fn column(
        &mut self,
        plan: &Plan<'_>,
        column: &DeclaredColumn<'_>,
        fields: &Fields,
    ) -> Result<Ts, String> {
        let kind = match &column.kind {
            Kind::Scalar(kind) => string(kind),
            Kind::Enum(ty) | Kind::Created(ty) => plan.type_name(ty),
        };
        let yes = || Ts::Atom("true".to_owned());
        let mut options = Vec::new();
        if !fields.names(&column.column.name) {
            options.push(("name".to_owned(), Ts::Atom(string(&column.column.name))));
        }
        if column.nullable {
            options.push(("nullable".to_owned(), yes()));
        }
        if column.primary_key {
            options.push(("primaryKey".to_owned(), yes()));
        }
        if column.array {
            options.push(("array".to_owned(), yes()));
        }
        if let Fill::Identity(identity) = column.fill {
            let generated = match identity {
                IdentityGeneration::Always => "always",
                IdentityGeneration::ByDefault => "byDefault",
            };
            options.push(("generated".to_owned(), Ts::Atom(string(generated))));
        }
        if let Some(values) = &column.values {
            let labels = values.iter().map(|label| Ts::Atom(string(label))).collect();
            options.push(("values".to_owned(), Ts::Array(labels)));
        }
        if let Some(ddl) = &column.column_type {
            let ddl = self.ddl(plan, ddl);
            options.push(("columnType".to_owned(), ddl));
        }
        match &column.fill {
            Fill::Value(literal) => {
                let value = self.value(literal, &column.kind);
                options.push(("defaultValue".to_owned(), Ts::Atom(value)));
            }
            Fill::Bound(literal) => {
                let bind = self.import("bind");
                let value = self.literal(literal);
                options.push((
                    "defaultExpr".to_owned(),
                    Ts::Atom(format!("{bind}({value})")),
                ));
            }
            Fill::Expr(expr) => {
                let expr = self.expr(expr)?;
                options.push(("defaultExpr".to_owned(), Ts::Atom(expr)));
            }
            Fill::Generated(expr, kind) => {
                let option = match kind {
                    GeneratedKind::Stored => "generatedStored",
                    GeneratedKind::Virtual => "generatedVirtual",
                };
                let expr = self.expr(expr)?;
                options.push((option.to_owned(), Ts::Atom(expr)));
            }
            Fill::AutoIncrement => options.push(("autoIncrement".to_owned(), yes())),
            Fill::None | Fill::Identity(_) => {}
        }
        if column.without_overlaps {
            options.push(("withoutOverlaps".to_owned(), yes()));
        }
        let callee = self.import("column");
        let mut args = vec![Ts::Atom(kind)];
        if !options.is_empty() {
            args.push(Ts::object(options));
        }
        Ok(Ts::Call(callee, args))
    }

    /// A stated column type as the module's `DataType` spells it.
    fn ddl(&mut self, plan: &Plan<'_>, ddl: &DdlType) -> Ts {
        match ddl {
            DdlType::Builtin(name) => Ts::Atom(string(name)),
            DdlType::Sized(name, length) => {
                self.data_type(name, vec![("length", Ts::Atom(length.to_string()))])
            }
            DdlType::Numeric(precision, scale) => {
                let mut sizes = vec![("precision", Ts::Atom(precision.to_string()))];
                if *scale != 0 {
                    sizes.push(("scale", Ts::Atom(scale.to_string())));
                }
                self.data_type("numeric", sizes)
            }
            DdlType::Interval(fields, precision) => {
                let mut options = Vec::new();
                if let Some(fields) = fields {
                    options.push(("fields", Ts::Atom(string(&fields.to_lower_camel_case()))));
                }
                if let Some(precision) = precision {
                    options.push(("precision", Ts::Atom(precision.to_string())));
                }
                self.data_type("interval", options)
            }
            DdlType::Enum(ty) | DdlType::Created(ty) => Ts::Atom(plan.type_name(ty)),
            DdlType::Domain { name, schema, base } => {
                let name = match schema {
                    Some(schema) => {
                        let class = self.import("TypeName");
                        Ts::Call(
                            format!("new {class}"),
                            vec![
                                Ts::Atom(string(name)),
                                Ts::object(vec![("schema".to_owned(), Ts::Atom(string(schema)))]),
                            ],
                        )
                    }
                    None => Ts::Atom(string(name)),
                };
                let base = self.ddl(plan, base);
                let class = self.import("DataType");
                Ts::Call(format!("{class}.domain"), vec![name, base])
            }
        }
    }

    fn data_type(&mut self, name: &str, options: Vec<(&str, Ts)>) -> Ts {
        let class = self.import("DataType");
        let options = options
            .into_iter()
            .map(|(option, value)| (option.to_owned(), value))
            .collect();
        Ts::Call(
            format!("new {class}"),
            vec![Ts::Atom(string(name)), Ts::object(options)],
        )
    }

    /// A literal default as the value the column's kind takes: an `i64`'s
    /// integer a `bigint`, a `decimal`'s a `Decimal`.
    fn value(&mut self, literal: &Literal, kind: &Kind) -> String {
        match (literal, kind) {
            (Literal::Int(int), Kind::Scalar("i64")) => format!("{int}n"),
            (Literal::Int(int), Kind::Scalar("decimal")) => self.decimal(&int.to_string()),
            (literal, _) => self.literal(literal),
        }
    }

    /// A literal as the value an operand binds it as: an integer past
    /// `integer`'s range a `bigint`, a decimal a `Decimal`.
    fn literal(&mut self, literal: &Literal) -> String {
        match literal {
            Literal::Int(int) if i32::try_from(*int).is_ok() => int.to_string(),
            Literal::Int(int) => format!("{int}n"),
            Literal::Float(text) => text.clone(),
            Literal::Decimal(text) => self.decimal(text),
            Literal::Text(text) => string(text),
            Literal::Bool(value) => value.to_string(),
        }
    }

    fn decimal(&mut self, text: &str) -> String {
        let class = self.import("Decimal");
        format!("new {class}({})", string(text))
    }

    /// `expr` as the TypeScript that builds it, an `Expr`.
    // [spec:pgorm:sem:codegen.models.columns]
    fn expr(&mut self, expr: &ColumnExpr) -> Result<String, String> {
        Ok(match expr {
            ColumnExpr::Column(name) => {
                let col = self.import("col");
                format!("{col}({})", string(name))
            }
            literal @ (ColumnExpr::Int(_)
            | ColumnExpr::BigInt(_)
            | ColumnExpr::Decimal(..)
            | ColumnExpr::Text(_)
            | ColumnExpr::Bool(_)) => {
                let bind = self.import("bind");
                format!("{bind}({})", self.operand(literal)?)
            }
            ColumnExpr::Null => {
                return Err(
                    "holds NULL in its expression, which a declared model has no word for".into(),
                );
            }
            ColumnExpr::Keyword(keyword) => {
                let function = self.import(match keyword {
                    CurrentKeyword::Date => "currentDate",
                    CurrentKeyword::Time => "currentTime",
                    CurrentKeyword::Timestamp => "currentTimestamp",
                });
                format!("{function}()")
            }
            ColumnExpr::Call(name, args) => {
                let func = self.import("func");
                let mut parts = vec![string(name)];
                for arg in args {
                    parts.push(self.operand(arg)?);
                }
                format!("{func}({})", parts.join(", "))
            }
            ColumnExpr::Cast(operand, cast) => {
                let receiver = self.expr(operand)?;
                let type_name = match &cast.schema {
                    None => string(&cast.name),
                    Some(schema) => {
                        let class = self.import("TypeName");
                        format!(
                            "new {class}({}, {{ schema: {} }})",
                            string(&cast.name),
                            string(schema)
                        )
                    }
                };
                let array = if cast.array { ", { array: true }" } else { "" };
                format!("{receiver}.cast({type_name}{array})")
            }
            ColumnExpr::Binary(left, op, right) => {
                let method = match op {
                    Operator::Add => "add",
                    Operator::Sub => "sub",
                    Operator::Mul => "mul",
                    Operator::Div => "div",
                    Operator::Mod => "mod",
                    Operator::Eq => "eq",
                    Operator::Ne => "ne",
                    Operator::Lt => "lt",
                    Operator::Gt => "gt",
                    Operator::Le => "lte",
                    Operator::Ge => "gte",
                    Operator::Concat => "concat",
                    Operator::And => "and",
                    Operator::Or => "or",
                };
                let receiver = self.expr(left)?;
                let other = match op {
                    Operator::And | Operator::Or => self.expr(right)?,
                    _ => self.operand(right)?,
                };
                format!("{receiver}.{method}({other})")
            }
            ColumnExpr::Not(operand) => format!("{}.not()", self.expr(operand)?),
            ColumnExpr::IsNull(operand, negated) => {
                let method = if *negated { "isNotNull" } else { "isNull" };
                format!("{}.{method}()", self.expr(operand)?)
            }
            ColumnExpr::Case(case) => self.case(case)?,
        })
    }

    /// `expr` where an operand is taken, a literal written as the value it
    /// is, which the call binds.
    fn operand(&mut self, expr: &ColumnExpr) -> Result<String, String> {
        match super::declared::literal(expr) {
            Some(literal) => Ok(self.literal(&literal)),
            None => self.expr(expr),
        }
    }

    fn case(&mut self, case: &CaseExpr) -> Result<String, String> {
        let case_when = self.import("caseWhen");
        let (condition, result) = &case.first;
        let mut text = format!(
            "{case_when}({}, {})",
            self.expr(condition)?,
            self.operand(result)?
        );
        for (condition, result) in &case.rest {
            text.push_str(&format!(
                ".when({}, {})",
                self.expr(condition)?,
                self.operand(result)?
            ));
        }
        if let Some(otherwise) = &case.otherwise {
            text.push_str(&format!(".else({})", self.operand(otherwise)?));
        }
        Ok(text)
    }

    /// The module: its header, its import, then `blocks`, a blank line
    /// between each.
    // [spec:pgorm:req:codegen.models.layout]
    fn module(&self, blocks: Vec<Vec<String>>) -> String {
        let mut out = format!(
            "// Declared models of a schema's tables, @generated by pgorm-codegen {}.\n",
            env!("CARGO_PKG_VERSION")
        );
        if !self.imports.is_empty() {
            let names: Vec<&str> = self.imports.iter().copied().collect();
            out.push('\n');
            out.push_str(&import(&names, PGORM));
            out.push('\n');
        }
        for block in blocks {
            out.push('\n');
            out.push_str(&block.join("\n"));
            out.push('\n');
        }
        out
    }
}

/// A type's `{ schema: .. }`.
fn schema_option(ty: &TypeRef) -> Ts {
    Ts::object(vec![("schema".to_owned(), Ts::Atom(string(&ty.schema)))])
}

/// `export const name = value;`.
fn export(name: &str, value: Ts) -> String {
    let head = format!("export const {name} = ");
    format!("{head}{};", value.render(0, head.len(), 1))
}
