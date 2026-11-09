//! The one seam between the pipeline builder and prqlc.
//!
//! Every `prqlc` import in pgorm lives in this file. The PL AST is not a
//! stable API, so the dependency is pinned exact (`=0.13.14`) and the rest of
//! the module works against the constructors below rather than the AST types;
//! a compiler bump is absorbed by rewriting this file alone.

use prqlc::pr::{
    BinaryExpr, Expr, ExprKind, FuncCall, Ident, InterpolateItem, Literal, ModuleDef, Pipeline,
    Range, Stmt, StmtKind, SwitchCase, UnaryExpr, VarDef, VarDefKind,
};
use prqlc::sql::Dialect;
use prqlc::{Options, Target};

pub(super) use prqlc::pr::{BinOp, UnOp};

/// The PL expression node the builder assembles.
// [spec:pgorm:def:pipeline.adapter+2]
pub(super) type PlExpr = Expr;

pub(super) fn ident(name: &str) -> PlExpr {
    Expr::new(ExprKind::Ident(Ident::from_name(name)))
}

pub(super) fn ident_in(path: Vec<String>, name: String) -> PlExpr {
    Expr::new(ExprKind::Ident(Ident { path, name }))
}

pub(super) fn call(name: &str, args: Vec<PlExpr>) -> PlExpr {
    Expr::new(ExprKind::FuncCall(FuncCall {
        name: Box::new(ident(name)),
        args,
        named_args: Default::default(),
    }))
}

pub(super) fn call_named(name: &str, args: Vec<PlExpr>, named: Vec<(&str, PlExpr)>) -> PlExpr {
    Expr::new(ExprKind::FuncCall(FuncCall {
        name: Box::new(ident(name)),
        args,
        named_args: named.into_iter().map(|(k, v)| (k.into(), v)).collect(),
    }))
}

pub(super) fn tuple(items: Vec<PlExpr>) -> PlExpr {
    Expr::new(ExprKind::Tuple(items))
}

/// One piece of an expression whose SQL pgorm writes itself: either text that
/// reaches the query verbatim, or a node prqlc renders in place.
// [spec:pgorm:def:pipeline.adapter+2]
#[derive(Debug, Clone)]
pub(super) enum Piece {
    Text(String),
    Node(PlExpr),
}

/// The expression `pieces` spell, projected under `alias`.
///
/// prqlc renders each interpolated node and emits the text between them
/// unchanged, so this is how pgorm writes SQL prqlc has no vocabulary for
/// while still letting prqlc resolve the column references inside it.
// [spec:pgorm:def:pipeline.adapter+2]
pub(super) fn assembled(pieces: Vec<Piece>, alias: Option<String>) -> PlExpr {
    let mut node = Expr::new(ExprKind::SString(
        pieces
            .into_iter()
            .map(|piece| match piece {
                Piece::Text(text) => InterpolateItem::String(text),
                Piece::Node(node) => InterpolateItem::Expr {
                    expr: Box::new(node),
                    format: None,
                },
            })
            .collect(),
    ));
    node.alias = alias;
    node
}

/// The arguments a call node applies its function to, or `None` for a node
/// that is not a call of a bare name.
// [spec:pgorm:def:pipeline.adapter+2]
pub(super) fn call_args(node: &PlExpr) -> Option<&[PlExpr]> {
    let ExprKind::FuncCall(call) = &node.kind else {
        return None;
    };
    Some(&call.args)
}

/// A sort key split into the expression it sorts by and whether it descends:
/// `desc` is a leading `-` and `asc` a leading `+`, both of which are marks on
/// the key rather than arithmetic, and neither of which belongs in the SQL
/// text an `ORDER BY` renders them into.
// [spec:pgorm:def:pipeline.adapter+2]
pub(super) fn sort_key(node: &PlExpr) -> (PlExpr, bool) {
    if let ExprKind::Unary(unary) = &node.kind {
        match unary.op {
            UnOp::Neg => return ((*unary.expr).clone(), true),
            UnOp::Add => return ((*unary.expr).clone(), false),
            _ => {}
        }
    }
    (node.clone(), false)
}

pub(super) fn array(items: Vec<PlExpr>) -> PlExpr {
    Expr::new(ExprKind::Array(items))
}

pub(super) fn nested(exprs: Vec<PlExpr>) -> PlExpr {
    Expr::new(ExprKind::Pipeline(Pipeline { exprs }))
}

pub(super) fn binary(left: PlExpr, op: BinOp, right: PlExpr) -> PlExpr {
    Expr::new(ExprKind::Binary(BinaryExpr {
        left: Box::new(left),
        op,
        right: Box::new(right),
    }))
}

pub(super) fn unary(op: UnOp, operand: PlExpr) -> PlExpr {
    Expr::new(ExprKind::Unary(UnaryExpr {
        op,
        expr: Box::new(operand),
    }))
}

pub(super) fn lit_int(value: i64) -> PlExpr {
    Expr::new(ExprKind::Literal(Literal::Integer(value)))
}

pub(super) fn lit_float(value: f64) -> PlExpr {
    Expr::new(ExprKind::Literal(Literal::Float(value)))
}

/// A string literal, written here rather than by the compiler.
///
/// prqlc hands a `Literal::String` to sqlparser, whose formatter is written
/// to serve every dialect at once: a quote already doubled, or one following
/// a backslash, is taken for pre-escaped text and emitted unchanged. That
/// second reading belongs to the backslash-escape regime, and the literal it
/// is written into is an ordinary one, where PostgreSQL gives a backslash no
/// meaning at all — so `\'` closes the string and the rest of the value is
/// read as SQL. Rendering the literal here keeps one regime end to end, and
/// the text reaches the emitted SQL through the pass-through node so that
/// nothing downstream escapes it a second time.
// [spec:pgorm:req:pipeline.params+4]
pub(super) fn lit_str(value: &str) -> PlExpr {
    Expr::new(ExprKind::SString(vec![InterpolateItem::String(
        string_literal(value),
    )]))
}

/// `value` as a self-contained PostgreSQL string literal.
///
/// A quote is doubled, which closes it under every setting. A backslash is
/// the one character whose meaning depends on `standard_conforming_strings`,
/// so a value carrying one is written as an `E''` string with its
/// backslashes doubled — the trigger `pgorm-query` renders on
/// ([spec:pgorm:req:sql.render.string-escape+1]) — and the literal then
/// denotes the value whichever way the server is configured.
// [spec:pgorm:req:pipeline.params+4]
fn string_literal(value: &str) -> String {
    let quoted = value.replace('\'', "''");
    if value.contains('\\') {
        format!("E'{}'", quoted.replace('\\', r"\\"))
    } else {
        format!("'{quoted}'")
    }
}

pub(super) fn lit_bool(value: bool) -> PlExpr {
    Expr::new(ExprKind::Literal(Literal::Boolean(value)))
}

pub(super) fn lit_null() -> PlExpr {
    Expr::new(ExprKind::Literal(Literal::Null))
}

/// A `$N` placeholder. `ExprKind::Param` passes through lowering verbatim,
/// so an index minted here that survives is the index the emitted SQL
/// carries — but the optimizer may prune the expression around it, which is
/// what the census in `into_sql` accounts for.
// [spec:pgorm:req:pipeline.params+4]
pub(super) fn param(index: usize) -> PlExpr {
    Expr::new(ExprKind::Param(index.to_string()))
}

pub(super) fn aliased(mut node: PlExpr, alias: String) -> PlExpr {
    node.alias = Some(alias);
    node
}

pub(super) fn case(arms: Vec<(PlExpr, PlExpr)>) -> PlExpr {
    Expr::new(ExprKind::Case(
        arms.into_iter()
            .map(|(condition, value)| SwitchCase {
                condition: Box::new(condition),
                value: Box::new(value),
            })
            .collect(),
    ))
}

pub(super) fn int_range(start: Option<i64>, end: Option<i64>) -> PlExpr {
    Expr::new(ExprKind::Range(Range {
        start: start.map(|v| Box::new(lit_int(v))),
        end: end.map(|v| Box::new(lit_int(v))),
    }))
}

/// The internal name of the `index`-th `let` binding: `table_N`.
///
/// The same namespace prqlc mints its own wrapping CTEs in; its namer steps
/// around taken names, so the two sequences never collide.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn binding_name(index: usize) -> String {
    format!("table_{index}")
}

fn binding_index(name: &str) -> Option<usize> {
    name.strip_prefix("table_")?.parse().ok()
}

/// Visit `node` and every expression nested inside it, outermost first.
///
/// The one recursion over the PL tree that the rewrites share, so a new AST
/// shape is taught to all of them at once rather than to whichever walker its
/// author remembered.
fn walk_mut(node: &mut PlExpr, visit: &mut impl FnMut(&mut PlExpr)) {
    visit(node);
    match &mut node.kind {
        ExprKind::Tuple(items) | ExprKind::Array(items) => {
            for item in items {
                walk_mut(item, visit);
            }
        }
        ExprKind::Pipeline(pipeline) => {
            for item in &mut pipeline.exprs {
                walk_mut(item, visit);
            }
        }
        ExprKind::Range(range) => {
            for bound in [&mut range.start, &mut range.end].into_iter().flatten() {
                walk_mut(bound, visit);
            }
        }
        ExprKind::Binary(node) => {
            walk_mut(&mut node.left, visit);
            walk_mut(&mut node.right, visit);
        }
        ExprKind::Unary(node) => walk_mut(&mut node.expr, visit),
        ExprKind::FuncCall(node) => {
            for arg in node.args.iter_mut().chain(node.named_args.values_mut()) {
                walk_mut(arg, visit);
            }
        }
        ExprKind::Case(arms) => {
            for arm in arms {
                walk_mut(&mut arm.condition, visit);
                walk_mut(&mut arm.value, visit);
            }
        }
        // The interpolated nodes of an expression pgorm wrote itself
        // ([`assembled`]) are ordinary expressions that prqlc resolves in
        // place, so every rewrite that reaches a column reference has to
        // reach the ones written inside one.
        ExprKind::SString(items) => {
            for item in items {
                if let InterpolateItem::Expr { expr, .. } = item {
                    walk_mut(expr, visit);
                }
            }
        }
        _ => {}
    }
}

/// Shift an embedded pipeline's frame so it keeps meaning inside its
/// consumer: every `$N` placeholder moves up by `params`, and every
/// reference to one of the pipeline's own `bindings` (of which it had
/// `binding_count`) moves up by `binding_offset`.
///
/// A binding is named by the *leading* segment of an identifier — bare where
/// a stage reads from it, qualifying where a column of it is referred to by
/// name ([`requalify`]) — so both spellings are renumbered.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn rebase(
    node: &mut PlExpr,
    params: usize,
    binding_count: usize,
    binding_offset: usize,
) {
    walk_mut(node, &mut |node| match &mut node.kind {
        ExprKind::Param(index) => {
            if let Ok(position) = index.parse::<usize>() {
                *index = (position + params).to_string();
            }
        }
        ExprKind::Ident(ident) => {
            let head = match ident.path.first_mut() {
                Some(head) => head,
                None => &mut ident.name,
            };
            if let Some(index) = binding_index(head)
                && index < binding_count
            {
                *head = binding_name(index + binding_offset);
            }
        }
        _ => {}
    });
}

/// Repoint the column references in `node` that `repoint` answers for, keyed
/// by the dotted path each reference carries.
///
/// A `Some((qualifier, name))` replaces the whole identifier, which is how a
/// reference written against a source follows its column into the binding
/// that now exposes it.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn requalify(node: &mut PlExpr, repoint: &dyn Fn(&str) -> Option<(String, String)>) {
    walk_mut(node, &mut |node| {
        if let Some(path) = column_path(node)
            && let Some((qualifier, name)) = repoint(&path)
            && let ExprKind::Ident(ident) = &mut node.kind
        {
            ident.path = vec![qualifier];
            ident.name = name;
        }
    });
}

/// Where a projected expression's name lands in the namespace prqlc resolves
/// `this` against.
///
/// prqlc files a column that is still qualified by one of the relation's
/// inputs under a submodule named for that input, and a name the pipeline
/// introduced at the top level of the same namespace. The two are ordered by
/// different keys — the input's position against the column's index — which
/// is what the builder's `Columns` exists to see coming.
// [spec:pgorm:req:pipeline.compose+2]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Projected<'a> {
    /// Qualified by this source, and filed under its submodule.
    Source(&'a str),
    /// A name the pipeline introduces, filed at the top level.
    Introduced,
    /// Left unnamed, so it is filed nowhere and `this` never expands it.
    Anonymous,
}

/// Classify one item of a projection by where its name will be filed.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn projected(node: &PlExpr) -> Projected<'_> {
    if node.alias.is_some() {
        return Projected::Introduced;
    }
    match &node.kind {
        ExprKind::Ident(ident) => match ident.path.first() {
            Some(source) => Projected::Source(source),
            None => Projected::Introduced,
        },
        _ => Projected::Anonymous,
    }
}

/// The bare name a call node names — a stage's verb (`select`, `derive`,
/// `sort`, `join`) or, on a projected expression, the function it calls
/// (`first`, `sum`) — or `None` for a node that is not a call of a bare name.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn stage_verb(node: &PlExpr) -> Option<&str> {
    let ExprKind::FuncCall(call) = &node.kind else {
        return None;
    };
    let ExprKind::Ident(ident) = &call.name.kind else {
        return None;
    };
    ident.path.is_empty().then_some(ident.name.as_str())
}

/// The name a `from` or `join` stage's relation answers to: the one that
/// qualifies its columns for every stage behind it.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn stage_source(node: &PlExpr) -> Option<&str> {
    let ExprKind::FuncCall(call) = &node.kind else {
        return None;
    };
    exposed_name(call.args.first()?)
}

/// The tuple a single-argument stage carries — a projection's items, a
/// sort's keys — reachable for replacement in place.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn tuple_items_mut(node: &mut PlExpr) -> Option<&mut Vec<PlExpr>> {
    let ExprKind::FuncCall(call) = &mut node.kind else {
        return None;
    };
    let ExprKind::Tuple(items) = &mut call.args.first_mut()?.kind else {
        return None;
    };
    Some(items)
}

/// The name an expression exposes to the stages after it: its alias, or the
/// last segment of the identifier it names. A projected column answers with
/// the name later stages address it by; a relation reference answers with the
/// name that qualifies its columns.
///
/// `None` is an expression left unnamed — which is precisely what `this`
/// cannot expand, because there is nothing to file it in the namespace under.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn exposed_name(node: &PlExpr) -> Option<&str> {
    if let Some(alias) = &node.alias {
        return Some(alias);
    }
    match &node.kind {
        ExprKind::Ident(ident) => Some(&ident.name),
        _ => None,
    }
}

/// The whole dotted path of a column reference — `fruit.name` — which is the
/// identity a projection item and a sort key naming it have in common.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn column_path(node: &PlExpr) -> Option<String> {
    let ExprKind::Ident(ident) = &node.kind else {
        return None;
    };
    let mut path = ident.path.join(".");
    if !path.is_empty() {
        path.push('.');
    }
    path.push_str(&ident.name);
    Some(path)
}

/// The column a sort key names, seen through the `desc`/`asc` marker that
/// wraps it.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn sort_key_path(key: &PlExpr) -> Option<String> {
    match &key.kind {
        ExprKind::Unary(node) => sort_key_path(&node.expr),
        _ => column_path(key),
    }
}

/// Repoint a sort key at an unqualified name, keeping its direction marker.
///
/// `false` when the key is not a column reference and so has no name to
/// move — an ordering computed from an expression cannot follow its relation
/// across a binding boundary.
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn rebind_sort_key(key: &mut PlExpr, name: &str) -> bool {
    match &mut key.kind {
        ExprKind::Unary(node) => rebind_sort_key(&mut node.expr, name),
        ExprKind::Ident(ident) => {
            ident.path.clear();
            ident.name = name.to_owned();
            true
        }
        _ => false,
    }
}

/// Visit `node` and every expression nested inside it, outermost first — the
/// read-only counterpart of [`walk_mut`], for the screens `into_sql` runs
/// before compiling.
///
/// It reaches two places [`walk_mut`] deliberately does not. A call's *name*
/// is an expression too, and while a rewrite must not repoint the verb a
/// stage applies, a screen has to see every name that leaves for prqlc. And
/// an interpolated string's pieces are covered under both spellings prqlc
/// has, `s` and `f`, so a node reachable through either is screened whether
/// or not this module has a constructor for it yet. The two shapes it cannot
/// reach — a `Func` body and an `Internal` operator name — are ones nothing
/// here builds.
fn walk(node: &PlExpr, visit: &mut impl FnMut(&PlExpr)) {
    visit(node);
    match &node.kind {
        ExprKind::Tuple(items) | ExprKind::Array(items) => {
            for item in items {
                walk(item, visit);
            }
        }
        ExprKind::Pipeline(pipeline) => {
            for item in &pipeline.exprs {
                walk(item, visit);
            }
        }
        ExprKind::Range(range) => {
            for bound in [&range.start, &range.end].into_iter().flatten() {
                walk(bound, visit);
            }
        }
        ExprKind::Binary(node) => {
            walk(&node.left, visit);
            walk(&node.right, visit);
        }
        ExprKind::Unary(node) => walk(&node.expr, visit),
        ExprKind::FuncCall(node) => {
            walk(&node.name, visit);
            for arg in node.args.iter().chain(node.named_args.values()) {
                walk(arg, visit);
            }
        }
        ExprKind::Case(arms) => {
            for arm in arms {
                walk(&arm.condition, visit);
                walk(&arm.value, visit);
            }
        }
        ExprKind::SString(items) | ExprKind::FString(items) => {
            for item in items {
                if let InterpolateItem::Expr { expr, .. } = item {
                    walk(expr, visit);
                }
            }
        }
        _ => {}
    }
}

/// Every alias set anywhere in `node`, in construction order.
pub(super) fn collect_aliases(node: &PlExpr, found: &mut Vec<String>) {
    walk(node, &mut |node| found.extend(node.alias.clone()));
}

/// Every identifier anywhere in `node` that prqlc will render as a name:
/// each alias, and each segment of each identifier reference — a schema, a
/// table, a binding, a column — in construction order.
///
/// This is the whole set of text pgorm hands the compiler to *quote*, which
/// is why the screen for a name that has no safe quoting
/// ([`PipelineError::UnquotableIdentifier`](super::PipelineError::UnquotableIdentifier))
/// reads it rather than the alias positions alone. The text between an
/// assembled expression's interpolated nodes is not among them: that is SQL
/// pgorm wrote itself, not a name a caller supplied.
pub(super) fn collect_identifiers(node: &PlExpr, found: &mut Vec<String>) {
    walk(node, &mut |node| {
        found.extend(node.alias.clone());
        if let ExprKind::Ident(ident) = &node.kind {
            found.extend(ident.path.iter().cloned());
            found.push(ident.name.clone());
        }
    });
}

/// Lower the assembled bindings and stages through prqlc: PL → RQ →
/// PostgreSQL SQL.
///
/// Each binding becomes a `let table_N = (...)` statement and the stages
/// become the `main` variable of an anonymous module — the same shape
/// `prqlc::prql_to_pl` produces for query text, so direct construction and
/// text compilation are interchangeable. prqlc lowers each referenced
/// binding to a CTE (or inlines it where SQL allows).
// [spec:pgorm:def:pipeline.adapter+2]
// [spec:pgorm:req:pipeline.compose+2]
pub(super) fn compile(bindings: Vec<Vec<PlExpr>>, stages: Vec<PlExpr>) -> Result<String, String> {
    let stmts = bindings
        .into_iter()
        .enumerate()
        .map(|(index, stages)| stmt(VarDefKind::Let, binding_name(index), stages))
        .chain([stmt(VarDefKind::Main, "main".into(), stages)])
        .collect();
    let module = ModuleDef {
        name: "Project".into(),
        stmts,
    };
    let options = Options::default()
        .with_target(Target::Sql(Some(Dialect::Postgres)))
        .no_format()
        .no_signature();
    prqlc::pl_to_rq(module)
        .and_then(|rq| prqlc::rq_to_sql(rq, &options))
        .map_err(|errors| errors.to_string())
}

fn stmt(kind: VarDefKind, name: String, stages: Vec<PlExpr>) -> Stmt {
    Stmt {
        kind: StmtKind::VarDef(VarDef {
            kind,
            name,
            value: Some(Box::new(nested(stages))),
            ty: None,
        }),
        span: None,
        annotations: vec![],
        doc_comment: None,
    }
}

#[cfg(test)]
pub(super) fn compile_text(prql: &str) -> Result<String, String> {
    let options = Options::default()
        .with_target(Target::Sql(Some(Dialect::Postgres)))
        .no_format()
        .no_signature();
    prqlc::compile(prql, &options).map_err(|errors| errors.to_string())
}
