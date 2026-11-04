//! What a window computes its columns over: partitioning, ordering and frame.
//!
//! Separate from the pipeline builder because a window spec is built and
//! finished on its own, and reaches the pipeline only as a finished value.

use super::adapter::{self, Piece, PlExpr};
use super::expr::{ExprList, nodes_of};

/// The calls prqlc renders wrongly and what pgorm renders in their place:
/// the PRQL name, the SQL function, which of the call's arguments reach it
/// in SQL order, and whether prqlc needs an authored frame before it goes
/// wrong.
///
/// Two things go wrong, and the last field is which. prqlc emits a frame
/// clause only for the calls its own standard library annotates as
/// frame-aware — the aggregates — and drops it silently from every other
/// window function. For `FIRST_VALUE` and `LAST_VALUE` the frame *is* the
/// answer, so a dropped one is a different query; for the ranking and offset
/// functions PostgreSQL ignores the frame either way, but they are written
/// the same way so that one rule covers the vocabulary rather than the subset
/// whose answer happens to move. None of that arises without an authored
/// frame, so those seven are left to prqlc until one is given.
///
/// `count` is the other thing, and it does not need a frame to go wrong:
/// PRQL's `count` counts the rows of the relation it is given, and prqlc's
/// standard library discards the counted column and emits `COUNT(*)`. pgorm's
/// surface separates the two answers — [`count_rows`](super::count_rows) is
/// `COUNT(*)` and [`count`](super::count) is `COUNT(expr)` — and they differ
/// on every row whose expression is null, so the counted one is written
/// wherever it appears, window or not.
///
/// The ranking functions take no argument: PRQL's `rank` names the column
/// being ranked, and `RANK()` has nowhere to put it.
// [spec:pgorm:sem:pipeline.window-frame]
// [spec:pgorm:sem:pipeline.count-argument]
const WRITTEN: [(&str, &str, &[usize], bool); 8] = [
    ("count", "COUNT", &[0], true),
    ("first", "FIRST_VALUE", &[0], false),
    ("lag", "LAG", &[1, 0], false),
    ("last", "LAST_VALUE", &[0], false),
    ("lead", "LEAD", &[1, 0], false),
    ("rank", "RANK", &[], false),
    ("rank_dense", "DENSE_RANK", &[], false),
    ("row_number", "ROW_NUMBER", &[], false),
];

/// What a [`window`](crate::pipeline::Pipeline::window) computes its columns over:
/// partitioning, ordering and frame.
///
/// Built by [`by`] (partition), [`sort_by`] (ordering) or [`over`] (neither),
/// then narrowed with [`rows`](Over::rows) or [`range`](Over::range).
///
/// The keys are `ExprList<'static>`, so a bound placeholder cannot enter a
/// window spec: `Over` erases the brand it was built from, and a partition
/// or ordering by a runtime value means nothing anyway.
///
/// ```compile_fail,E0521
/// use pgorm::pipeline::{ExprOps, Pipeline, by};
/// use pgorm::tests_cfg::cake::{self, Column as C};
///
/// let _ = Pipeline::from(cake::Entity).filter_with(|binder| {
///     let _smuggled = by(binder.bind(1_i32));
///     C::Id.gt(1)
/// });
/// ```
// [spec:pgorm:req:pipeline.surface+4]
#[derive(Debug, Clone, Default)]
pub struct Over {
    partition: Vec<PlExpr>,
    sort: Vec<PlExpr>,
    frame: Option<(&'static str, Option<i64>, Option<i64>)>,
}

/// A window over the whole relation, unpartitioned and unordered.
pub fn over() -> Over {
    Over::default()
}

/// A window partitioned by these keys: PRQL's `group`, SQL's `PARTITION BY`.
pub fn by(keys: impl ExprList<'static>) -> Over {
    over().by(keys)
}

/// A window ordered by these keys ([`desc`](super::ExprOps::desc) marks one
/// descending).
pub fn sort_by(keys: impl ExprList<'static>) -> Over {
    over().sort_by(keys)
}

impl Over {
    /// `PARTITION BY` these keys.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn by(mut self, keys: impl ExprList<'static>) -> Self {
        self.partition = nodes_of(keys);
        self
    }

    /// `ORDER BY` these keys within the window.
    ///
    /// Without a partition the sort is a real pipeline stage, so it also
    /// orders the output — PRQL semantics, kept rather than hidden.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn sort_by(mut self, keys: impl ExprList<'static>) -> Self {
        self.sort = nodes_of(keys);
        self
    }

    /// A `ROWS BETWEEN ... AND ...` frame, in rows relative to the current
    /// row: `0` is the current row, negative precedes, positive follows, and
    /// `None` leaves that side unbounded.
    ///
    /// The two bounds are independent, so every direction is expressible, and
    /// the frame reaches the emitted SQL for every window function — including
    /// the ones the compiler renders without it.
    // [spec:pgorm:sem:pipeline.window-frame]
    pub fn rows(mut self, start: Option<i64>, end: Option<i64>) -> Self {
        self.frame = Some(("rows", start, end));
        self
    }

    /// A `RANGE BETWEEN ... AND ...` frame, in values, with bounds read as
    /// in [`rows`](Over::rows).
    // [spec:pgorm:sem:pipeline.window-frame]
    pub fn range(mut self, start: Option<i64>, end: Option<i64>) -> Self {
        self.frame = Some(("range", start, end));
        self
    }

    /// The stages this window becomes, given the columns it computes and the
    /// ordering the pipeline already carried.
    ///
    /// Columns whose frame prqlc would drop are rendered here instead, as a
    /// plain `derive` of expressions that spell their own `OVER (...)`; the
    /// rest keep prqlc's `window` transform, which still reads an
    /// unpartitioned window's ordering from the pipeline — hence `inherited`,
    /// which is that ordering and is what the written clause has to agree
    /// with.
    // [spec:pgorm:sem:pipeline.window-frame]
    pub(super) fn wrap(self, columns: Vec<PlExpr>, inherited: Vec<PlExpr>) -> Vec<PlExpr> {
        let clause = self.over_clause(&inherited);
        let framed = self.frame.is_some();
        let (mut written, mut deferred) = (Vec::new(), Vec::new());
        for node in columns {
            match rewritten(&node, &clause, framed) {
                Some(spelled) => written.push(spelled),
                None => deferred.push(node),
            }
        }
        let Over {
            partition,
            sort,
            frame,
        } = self;

        let sort_call =
            (!sort.is_empty()).then(|| adapter::call("sort", vec![adapter::tuple(sort)]));
        let mut stages = Vec::new();
        if deferred.is_empty() {
            // An unpartitioned window's ordering is a pipeline stage whether
            // or not prqlc is left anything to read it.
            if partition.is_empty() {
                stages.extend(sort_call);
            }
        } else {
            let derive_call = adapter::call("derive", vec![adapter::tuple(deferred)]);
            let window_call = match frame {
                Some((kind, start, end)) => adapter::call_named(
                    "window",
                    vec![derive_call],
                    vec![(kind, adapter::int_range(start, end))],
                ),
                None => adapter::call("window", vec![derive_call]),
            };
            if partition.is_empty() {
                stages.extend(sort_call);
                stages.push(window_call);
            } else {
                let body = match sort_call {
                    Some(sort_call) => adapter::nested(vec![sort_call, window_call]),
                    None => window_call,
                };
                stages.push(adapter::call(
                    "group",
                    vec![adapter::tuple(partition), body],
                ));
            }
        }
        if !written.is_empty() {
            stages.push(adapter::call("derive", vec![adapter::tuple(written)]));
        }
        stages
    }

    /// The `OVER (...)` clause this window spells, as text around the keys
    /// prqlc renders.
    // [spec:pgorm:sem:pipeline.window-frame]
    fn over_clause(&self, inherited: &[PlExpr]) -> Vec<Piece> {
        let mut pieces = vec![Piece::Text(" OVER (".to_owned())];
        let mut written = false;
        if !self.partition.is_empty() {
            push_text(&mut pieces, "PARTITION BY ");
            push_keys(&mut pieces, &self.partition);
            written = true;
        }
        let ordering = self.ordering_keys(inherited);
        if !ordering.is_empty() {
            push_text(
                &mut pieces,
                if written { " ORDER BY " } else { "ORDER BY " },
            );
            push_keys(&mut pieces, ordering);
            written = true;
        }
        if let Some((kind, start, end)) = self.frame {
            if written {
                push_text(&mut pieces, " ");
            }
            push_text(&mut pieces, &frame_clause(kind, start, end));
        }
        push_text(&mut pieces, ")");
        pieces
    }

    /// What the `OVER` clause orders by: this window's own keys, or — for an
    /// unpartitioned one, which is the only kind prqlc lets read past itself —
    /// the ordering the pipeline already carried.
    // [spec:pgorm:sem:pipeline.window-frame]
    fn ordering_keys<'keys>(&'keys self, inherited: &'keys [PlExpr]) -> &'keys [PlExpr] {
        if !self.sort.is_empty() {
            &self.sort
        } else if self.partition.is_empty() {
            inherited
        } else {
            &[]
        }
    }
}

/// The columns of a stage that is not a window, with every call pgorm writes
/// without needing a frame written out and `clause` appended.
///
/// `clause` is the `OVER ()` prqlc gives an aggregate used outside a
/// grouping — an implicit window over the whole relation — and empty inside
/// an `aggregate`, where the grouping says what the aggregate ranges over.
/// The calls that are only wrong under a frame are left to prqlc here: this
/// stage authored none.
// [spec:pgorm:sem:pipeline.count-argument]
pub(super) fn written_aggregates(nodes: Vec<PlExpr>, clause: &str) -> Vec<PlExpr> {
    let clause: Vec<Piece> = match clause.is_empty() {
        true => Vec::new(),
        false => vec![Piece::Text(clause.to_owned())],
    };
    nodes
        .into_iter()
        .map(|node| rewritten(&node, &clause, false).unwrap_or(node))
        .collect()
}

/// `node` rewritten to spell its own SQL, or `None` when prqlc renders it
/// correctly as it stands. `framed` is whether the enclosing window authored
/// a frame, which is what the calls prqlc renders frame-blind are waiting
/// for.
///
/// The call is written out rather than interpolated whole: prqlc appends an
/// `OVER (...)` of its own to any window function it renders, so a call left
/// intact inside the assembled expression would carry two.
// [spec:pgorm:sem:pipeline.window-frame]
// [spec:pgorm:sem:pipeline.count-argument]
fn rewritten(node: &PlExpr, clause: &[Piece], framed: bool) -> Option<PlExpr> {
    // A window function call has the shape a stage does — a bare name applied
    // to arguments — so the same reader answers for both.
    let name = adapter::stage_verb(node)?;
    let (_, sql, order, unframed) = WRITTEN.iter().find(|(prql, ..)| *prql == name)?;
    if !(framed || *unframed) {
        return None;
    }
    let args = adapter::call_args(node)?;
    if counts_rows(name, args) {
        return None;
    }
    let mut pieces = vec![Piece::Text(format!("{sql}("))];
    for (position, arg) in order
        .iter()
        .filter_map(|index| args.get(*index))
        .enumerate()
    {
        if position > 0 {
            push_text(&mut pieces, ", ");
        }
        pieces.push(Piece::Node(arg.clone()));
    }
    push_text(&mut pieces, ")");
    pieces.extend(clause.iter().cloned());
    let alias = adapter::exposed_name(node).map(str::to_owned);
    Some(adapter::assembled(pieces, alias))
}

/// Whether this call is PRQL's `count this` — the row count, which pgorm
/// spells [`count_rows`](super::count_rows) and prqlc already renders as
/// `COUNT(*)`, frame included.
///
/// `this` is the relation itself, so it is the one argument `COUNT` has
/// nowhere to put; writing the call out would put the name in the SQL.
// [spec:pgorm:sem:pipeline.count-argument]
fn counts_rows(name: &str, args: &[PlExpr]) -> bool {
    name == "count"
        && args
            .first()
            .and_then(adapter::column_path)
            .is_some_and(|path| path == "this")
}

/// Append `text`, merging it into the text already at the end so that the
/// assembled expression holds one piece per run rather than one per call.
// [spec:pgorm:sem:pipeline.window-frame]
fn push_text(pieces: &mut Vec<Piece>, text: &str) {
    match pieces.last_mut() {
        Some(Piece::Text(last)) => last.push_str(text),
        _ => pieces.push(Piece::Text(text.to_owned())),
    }
}

/// Append a comma-separated key list, each key stripped of its sort mark and
/// carrying `DESC` as text where it had one.
// [spec:pgorm:sem:pipeline.window-frame]
fn push_keys(pieces: &mut Vec<Piece>, keys: &[PlExpr]) {
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            push_text(pieces, ", ");
        }
        let (key, descending) = adapter::sort_key(key);
        pieces.push(Piece::Node(key));
        if descending {
            push_text(pieces, " DESC");
        }
    }
}

/// The frame clause for bounds read relative to the current row, which is how
/// [`rows`](Over::rows) and [`range`](Over::range) document them: `0` is the
/// current row, a negative offset precedes it, a positive one follows, and
/// `None` is unbounded on that side.
// [spec:pgorm:sem:pipeline.window-frame]
fn frame_clause(kind: &str, start: Option<i64>, end: Option<i64>) -> String {
    let units = if kind == "range" { "RANGE" } else { "ROWS" };
    let start = frame_bound(start, "UNBOUNDED PRECEDING");
    let end = frame_bound(end, "UNBOUNDED FOLLOWING");
    format!("{units} BETWEEN {start} AND {end}")
}

/// One frame bound, `unbounded` naming the side it sits on.
// [spec:pgorm:sem:pipeline.window-frame]
fn frame_bound(offset: Option<i64>, unbounded: &str) -> String {
    match offset {
        None => unbounded.to_owned(),
        Some(0) => "CURRENT ROW".to_owned(),
        Some(offset) if offset < 0 => format!("{} PRECEDING", offset.unsigned_abs()),
        Some(offset) => format!("{offset} FOLLOWING"),
    }
}
