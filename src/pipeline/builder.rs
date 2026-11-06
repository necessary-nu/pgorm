//! The pipeline itself: a source and a sequence of whole transforms.

use super::sources::Named;
use std::ops::RangeInclusive;

use pgorm_query::{AliasName, IntoName, Name, SqlName, Value};

use crate::EntityTrait;
use crate::entity::default_alias;

use super::adapter::{self, PlExpr, Projected};
use super::binder::Binder;
use super::expr::{Expr, ExprList, nodes_of};
use super::naming;
use super::sets;
use super::window::{self, Over};

/// The window prqlc gives an aggregate used outside a grouping: the whole
/// relation, unpartitioned and unordered. A stage that authored no window of
/// its own writes this one, so a call pgorm renders itself keeps the reading
/// prqlc gives the calls beside it.
// [spec:pgorm:sem:pipeline.count-argument]
const IMPLICIT_WINDOW: &str = " OVER ()";

/// How prqlc will expand `this` — the whole-relation reference
/// [`distinct`](Pipeline::distinct) deduplicates on — over the columns
/// accumulated so far.
///
/// prqlc resolves `this` by walking a namespace tree rather than the
/// relation's column list, and orders that tree by two incomparable keys: a
/// column still qualified by an input sits under a submodule ordered by the
/// *input's position*, while a name the pipeline introduced sits at the top
/// level ordered by its *column index*. The order that falls out is the
/// declared one only while the columns all answer to the same key, so the
/// pipeline tracks which of those shapes it is in.
// [spec:pgorm:req:pipeline.compose+1]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Columns {
    /// The sources' own columns, in source order, optionally followed by
    /// names this pipeline introduced — which `this` also expands last, so
    /// the declared order survives.
    Sourced { introduced: bool },
    /// Only names this pipeline introduced, in the order they were declared.
    Introduced,
    /// Source columns and introduced names interleaved, or more than one
    /// source projected explicitly. `this` regroups these by source, so the
    /// declared order is not what comes out.
    Regrouped,
}

impl Columns {
    /// The shape of a projection, read off the items it lists.
    fn of(items: &[PlExpr]) -> Self {
        let mut source: Option<&str> = None;
        let mut introduced = false;
        for item in items {
            match adapter::projected(item) {
                Projected::Introduced => introduced = true,
                Projected::Source(name) => match source {
                    // A second source projected explicitly: `this` would
                    // emit each source's columns together, whatever order
                    // the list interleaved them in.
                    Some(first) if first != name => return Columns::Regrouped,
                    Some(_) => {}
                    None => source = Some(name),
                },
                // Nameless, so `this` never expands it and it takes no
                // position among the ones it does — this question is about
                // the order of the columns that do have one. Reaching the
                // column at all is a matter of giving it a name, which
                // `distinct` does before it reads this shape back.
                Projected::Anonymous => {}
            }
        }
        match (source, introduced) {
            (Some(_), false) => Columns::Sourced { introduced: false },
            (None, _) => Columns::Introduced,
            (Some(_), true) => Columns::Regrouped,
        }
    }

    /// Names appended after the columns already present — `derive`, `window`,
    /// the aggregates of an `aggregate`. They land last either way, so they
    /// only ever add a trailing run.
    fn with_introduced(self) -> Self {
        match self {
            Columns::Sourced { .. } => Columns::Sourced { introduced: true },
            other => other,
        }
    }

    /// Whether `this` would expand these columns in the order they were
    /// declared.
    fn ordered(self) -> bool {
        self != Columns::Regrouped
    }
}

/// A relation-to-relation query pipeline in PRQL's shape.
///
/// [`from`](Pipeline::from) is the only way in, so a sourceless pipeline is
/// unrepresentable; every method appends one whole transform, so a
/// half-formed stage is unrepresentable too. Clause placement is the
/// compiler's job: a [`filter`](Pipeline::filter) lands in `WHERE`, `HAVING`
/// or a wrapping subquery according to where it sits in the pipeline, not
/// according to which method was called.
///
/// Each transform comes in two forms. The plain one takes expressions by
/// value, which is every query whose constants are written in the source:
///
/// ```
/// # use pgorm::pipeline::{ExprOps, Pipeline};
/// # use pgorm::tests_cfg::cake::{self, Column as C};
/// let (sql, values) = Pipeline::from(cake::Entity)
///     .filter(C::Id.gt(10))
///     .sort(C::Name)
///     .take(5)
///     .into_sql()?;
/// assert_eq!(sql, "SELECT * FROM cake WHERE id > 10 ORDER BY name LIMIT 5");
/// assert!(values.0.is_empty());
/// # Ok::<_, pgorm::pipeline::PipelineError>(())
/// ```
///
/// The `_with` one takes a closure and hands it the [`Binder`], which is the
/// only door a runtime value enters by:
///
/// ```
/// # use pgorm::pipeline::{ExprOps, Pipeline};
/// # use pgorm::tests_cfg::cake::{self, Column as C};
/// let (sql, values) = Pipeline::from(cake::Entity)
///     .filter_with(|binder| C::Id.gt(binder.bind(10_i32)))
///     .into_sql()?;
/// assert_eq!(sql, "SELECT * FROM cake WHERE id > $1");
/// assert_eq!(values.0.len(), 1);
/// # Ok::<_, pgorm::pipeline::PipelineError>(())
/// ```
// [spec:pgorm:req:pipeline.surface+4]
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub(super) bindings: Vec<Vec<PlExpr>>,
    pub(super) stages: Vec<PlExpr>,
    pub(super) values: Vec<Value>,
    /// The first stage of *this* pipeline that replaced its sources' own
    /// column namespaces — `select`, `group().aggregate()`, `intersect` or
    /// `remove` — or `None` while every source is still addressable.
    ///
    /// [`select_sources`](Pipeline::select_sources) refuses a reshaped
    /// pipeline by this name. The flag is deliberately per-pipeline: an
    /// embedded pipeline that reshaped itself is a table-like relation whose
    /// resulting columns the CTE boundary re-exposes, so embedding does not
    /// propagate it.
    // [spec:pgorm:sem:pipeline.select-sources+5]
    pub(super) reshaped: Option<&'static str>,
    /// Whether the stages accumulated so far end in a deduplicating `group`
    /// ([`distinct`](Pipeline::distinct)) that no binding has absorbed yet.
    ///
    /// prqlc cannot take a set operation directly off a grouped relation —
    /// it reads the group's arity as the tuple it keys on rather than the
    /// columns the relation projects, and refuses the combination for a
    /// column-count mismatch that is not there. Hoisting the grouped stages
    /// into their own binding first settles the arity at the CTE boundary,
    /// so the flag records only whether that hoist is still owed.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) deduped: bool,
    /// How prqlc would expand `this` over the columns accumulated so far.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) columns: Columns,
    /// Whether the relation still projects a wildcard — a column prqlc carries
    /// as `*` because the source's schema is not something it can see.
    ///
    /// Every table is read that way, and a projection is what replaces it, so
    /// this is true until a `select` or an `aggregate` says what the columns
    /// are. A deduplication over such a relation keys on the wildcard too,
    /// which is what [`bare_star_key`](naming::bare_star_key) then weighs.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) starred: bool,
    /// Whether the stages end in a deduplication that keyed on a star prqlc
    /// has no legal spelling for, and that no binding has absorbed yet.
    ///
    /// prqlc renders the deduplication as plain `SELECT DISTINCT *` while its
    /// key is exactly the relation's frame, and falls through to
    /// `DISTINCT ON (<key>)` as soon as the two differ — which any stage
    /// landing in the same query makes them. The star is in that key, and
    /// unqualified it is a syntax error, so the deduplication has to be the
    /// last thing in its own query: the next stage hoists it into a binding,
    /// where the frame settles and plain `DISTINCT` stands.
    ///
    /// The hoist is owed rather than performed so that a deduplication nothing
    /// follows still renders as the one query it always was.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) bare_star_key: bool,
    /// Whether a row range is waiting in front of a
    /// [`distinct`](Pipeline::distinct).
    ///
    /// prqlc lowers every `take` to the same transform and then renders the
    /// deduplicating group's own `take 1` and the pipeline's range take into
    /// one query, so the `LIMIT`/`OFFSET` lands *after* the deduplication
    /// rather than before it — a different set of rows, not a different
    /// order. Settling the range at a CTE boundary first keeps the two takes
    /// in separate queries, in the order the stages were written.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) ranged: bool,
    /// The source names a settle took out of scope, and the binding their
    /// columns answer under now.
    ///
    /// Every stage appended from here is repointed through it, which is what
    /// keeps a `col(source, column)` reference written after a
    /// [`distinct`](Pipeline::distinct) resolving.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) settled: naming::Settled,
    /// The `sort` stage that still describes the relation's order, or `None`
    /// while nothing has ordered it since the columns were last replaced.
    ///
    /// PRQL's `group` resets the order, and prqlc's flattener deletes the
    /// standalone `sort` that a group follows — so the ordering a
    /// [`distinct`](Pipeline::distinct) inherits has to be written again on
    /// the far side of the deduplication. That is exactly the `distinct` then
    /// `sort` shape, which renders the `DISTINCT` in a binding with the
    /// `ORDER BY` outside it, where PostgreSQL's rule that an ordering under
    /// `DISTINCT` must be projected cannot bite.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) ordering: Option<PlExpr>,
    /// The set operation the stages end in, while no binding has absorbed
    /// its result yet.
    ///
    /// prqlc renders a stage after a set operation into the same query as
    /// the operation's first operand wherever it can: a projection lands on
    /// that operand's `SELECT` alone, so the other operand keeps a wildcard
    /// and the arms do not line up, and a column it prunes for a later
    /// stage is pruned from the first operand only. Under `INTERSECT` and
    /// `EXCEPT` the pruning is wrong even where both operands are pruned,
    /// since the operation then compares fewer columns. A binding is the
    /// bracket prqlc keeps: the next stage that reads the combined relation
    /// settles the stages into one first, so it reads the whole result.
    /// Another set operation and a deduplication do not read it that way —
    /// the first is a chain [`sets`] brackets where it must, the second folds
    /// into `UNION DISTINCT` — and leave the settle owed.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) combined: bool,
    /// The columns a reader of the rows reads, each with the type it is read
    /// as, which [`into_sql`](Pipeline::into_sql) ends the pipeline in a
    /// projection of: `None` while it is compiled as written. Set by
    /// [`read_columns`](Pipeline::read_columns), and only where some column
    /// is read through a cast.
    // [spec:pgorm:sem:pipeline.described+1]
    pub(super) read: Option<Vec<(String, Option<pgorm_query::TypeName>)>>,
}

/// Which rows a [`join`](Pipeline::join) keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinSide {
    /// `JOIN`
    Inner,
    /// `LEFT JOIN`
    Left,
    /// `RIGHT JOIN`
    Right,
    /// `FULL JOIN`
    Full,
}

impl JoinSide {
    fn keyword(self) -> &'static str {
        match self {
            JoinSide::Inner => "inner",
            JoinSide::Left => "left",
            JoinSide::Right => "right",
            JoinSide::Full => "full",
        }
    }
}

/// A relation a pipeline can read: the argument of [`from`](Pipeline::from),
/// [`join`](Pipeline::join) and the set operations.
///
/// An entity brings its own table name and schema, so it is the ordinary
/// source; [`alias`](pgorm_query::alias) and [`Name`] name a table no
/// entity describes; and a whole [`Pipeline`] is a relation too, embedded as
/// a `let`-bound subrelation.
// [spec:pgorm:sem:pipeline.qualify+4]
pub trait IntoSource {
    /// The relation, ready to embed.
    fn into_source(self) -> Source;

    /// Read this relation under a name of your own, so a pipeline can meet
    /// the same table twice.
    ///
    /// The name is the relation's only name from then on, as in SQL: an
    /// aliased entity no longer answers to its table name, and every
    /// reference to its columns goes through the name: [`Named::col`] for
    /// an entity's column, compared as the entity types it, or
    /// [`col`](super::col), which only qualifies. The returned [`Named`]
    /// remembers what it wraps, which is how the same spelling restates an
    /// entity source to [`select_sources`](Pipeline::select_sources).
    ///
    /// ```
    /// # use pgorm::pipeline::{ExprOps, IntoSource, JoinSide, Pipeline, alias, col};
    /// # use pgorm::tests_cfg::fruit::{self, Column as F};
    /// let peer = alias("peer");
    /// let (sql, _) = Pipeline::from(fruit::Entity)
    ///     .join(
    ///         JoinSide::Inner,
    ///         fruit::Entity.named(peer),
    ///         F::CakeId.eq(col(peer, alias("cake_id"))),
    ///     )
    ///     .select((F::Name, col(peer, alias("name"))))
    ///     .into_sql()?;
    /// assert_eq!(
    ///     sql,
    ///     "SELECT fruit.name AS _expr_0, peer.name FROM fruit \
    ///      INNER JOIN fruit AS peer ON fruit.cake_id = peer.cake_id"
    /// );
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    // [spec:pgorm:sem:pipeline.self-join+1]
    // [spec:pgorm:sem:pipeline.select-sources+5]
    fn named(self, name: impl Into<AliasName>) -> Named<Self>
    where
        Self: Sized,
    {
        Named {
            relation: self,
            name: name.into().as_str().to_owned(),
        }
    }
}

/// A relation on its way into a pipeline, made by [`IntoSource`].
///
/// A table travels as its identifier; a pipeline travels whole — stages,
/// bindings and bound values — and is `let`-bound by the consumer. The
/// contents are not constructible outside the pipeline module, so the set of
/// relation shapes is closed.
// [spec:pgorm:req:pipeline.compose+1]
#[derive(Debug)]
pub struct Source {
    kind: SourceKind,
    pub(super) alias: Option<String>,
}

#[derive(Debug)]
enum SourceKind {
    Table(PlExpr),
    Pipeline(Box<Pipeline>),
}

fn table_source(node: PlExpr) -> Source {
    Source {
        kind: SourceKind::Table(node),
        alias: None,
    }
}

/// A relation already carried as a [`Source`] passes through unchanged.
// [spec:pgorm:sem:pipeline.self-join+1]
impl IntoSource for Source {
    fn into_source(self) -> Source {
        self
    }
}

/// The instance's own schema and table, read under the default's table name
/// where the instance's differs: an entity column qualifies itself by that
/// name, so the relation has to answer to it.
// [spec:pgorm:sem:pipeline.qualify+4]
impl<E: EntityTrait> IntoSource for E {
    fn into_source(self) -> Source {
        let mut source = table_source(match self.schema_name() {
            Some(schema) => {
                adapter::ident_in(vec![schema.to_owned()], self.table_name().to_owned())
            }
            None => adapter::ident(self.table_name()),
        });
        source.alias = default_alias(&self).map(|alias| SqlName::to_string(&*alias));
        source
    }
}

// [spec:pgorm:sem:pipeline.qualify+4]
impl IntoSource for AliasName {
    fn into_source(self) -> Source {
        table_source(adapter::ident(self.as_str()))
    }
}

// [spec:pgorm:sem:pipeline.qualify+4]
impl IntoSource for Name {
    fn into_source(self) -> Source {
        table_source(adapter::ident(&SqlName::to_string(&*self)))
    }
}

/// A whole pipeline is a relation. Embedding consumes it by value, so its
/// bound values travel with its placeholders and the pair stays aligned; an
/// expression cannot make the same crossing alone
/// (`[spec:pgorm:req:pipeline.params+4]`).
// [spec:pgorm:req:pipeline.compose+1]
impl IntoSource for Pipeline {
    fn into_source(self) -> Source {
        Source {
            kind: SourceKind::Pipeline(Box::new(self)),
            alias: None,
        }
    }
}

/// A pipeline that has been grouped and is waiting for its aggregates.
///
/// [`Pipeline::group`] cannot produce a pipeline on its own — PRQL's `group`
/// is a transform over a body, and a grouping with nothing aggregated is not
/// a relation — so the only way back to a [`Pipeline`] is
/// [`aggregate`](Grouped::aggregate).
// [spec:pgorm:req:pipeline.surface+4]
#[derive(Debug, Clone)]
pub struct Grouped {
    pipeline: Pipeline,
    keys: Vec<PlExpr>,
}

impl Grouped {
    /// Aggregate each group; the resulting relation carries the keys
    /// followed by the aggregates.
    // [spec:pgorm:req:pipeline.surface+4]
    pub fn aggregate(self, aggregates: impl ExprList<'static>) -> Pipeline {
        let nodes = nodes_of(aggregates);
        self.finish(nodes)
    }

    /// Aggregate each group, with runtime values bound in the closure.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn aggregate_with<F, const N: usize>(mut self, f: F) -> Pipeline
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> [Expr<'brand>; N],
    {
        let nodes = {
            let aggregates = f(&mut Binder::new(&mut self.pipeline.values));
            aggregates.into_iter().map(|expr| expr.node).collect()
        };
        self.finish(nodes)
    }

    fn finish(mut self, aggregates: Vec<PlExpr>) -> Pipeline {
        for key in &mut self.keys {
            self.pipeline.settled.requalify(key);
        }
        // The relation is the keys followed by the aggregates, which are
        // introduced names and so trail them either way.
        let columns = Columns::of(&self.keys).with_introduced();
        self.pipeline.ordering = None;
        // An aggregation says what every resulting column is, so no wildcard
        // survives it.
        self.pipeline.starred = false;
        // Inside an `aggregate` the grouping says what an aggregate ranges
        // over, so a written one carries no `OVER` clause of its own.
        let aggregates = window::written_aggregates(aggregates, "");
        let stage = adapter::call(
            "group",
            vec![
                adapter::tuple(self.keys),
                adapter::call("aggregate", vec![adapter::tuple(aggregates)]),
            ],
        );
        let mut grouped = self.pipeline.stage(stage).reshaping("group().aggregate()");
        grouped.columns = columns;
        grouped
    }
}

impl Pipeline {
    /// Start a pipeline from a relation: an entity (schema and all), a table
    /// named some other way, or another pipeline embedded whole.
    // [spec:pgorm:req:pipeline.surface+4]
    pub fn from(source: impl IntoSource) -> Self {
        let mut pipeline = Pipeline {
            bindings: Vec::new(),
            stages: Vec::new(),
            values: Vec::new(),
            reshaped: None,
            deduped: false,
            columns: Columns::Sourced { introduced: false },
            starred: false,
            bare_star_key: false,
            ranged: false,
            settled: naming::Settled::default(),
            ordering: None,
            combined: false,
            read: None,
        };
        let reference = pipeline.embed(source.into_source());
        pipeline.stages.push(adapter::call("from", vec![reference]));
        pipeline
    }

    /// Start a pipeline from a schema-qualified table no entity describes.
    // [spec:pgorm:sem:pipeline.qualify+4]
    pub fn from_schema(schema: impl IntoName, table: impl IntoName) -> Self {
        let source = adapter::ident_in(
            vec![SqlName::to_string(&*schema.into_name())],
            SqlName::to_string(&*table.into_name()),
        );
        Pipeline {
            bindings: Vec::new(),
            stages: vec![adapter::call("from", vec![source])],
            values: Vec::new(),
            reshaped: None,
            deduped: false,
            columns: Columns::Sourced { introduced: false },
            starred: true,
            bare_star_key: false,
            ranged: false,
            settled: naming::Settled::default(),
            ordering: None,
            combined: false,
            read: None,
        }
    }

    /// Merge an embedded relation into this pipeline; the returned expression
    /// is how the stages refer to it.
    ///
    /// A table is its identifier. A pipeline becomes the next `let` binding:
    /// its values append to this pipeline's and its `$N` placeholders shift
    /// by the count already bound here, so position `N` in the merged SQL is
    /// still position `N` in the merged values; its own bindings come along,
    /// renumbered past the ones already present, with every internal
    /// reference rewritten to match.
    ///
    /// A [`named`](IntoSource::named) relation carries its alias onto the
    /// reference, which is what lets the same table appear twice.
    // [spec:pgorm:req:pipeline.compose+1]
    // [spec:pgorm:sem:pipeline.self-join+1]
    fn embed(&mut self, source: Source) -> PlExpr {
        let Source { kind, alias } = source;
        let reference = self.embed_kind(kind);
        match alias {
            Some(name) => adapter::aliased(reference, name),
            None => reference,
        }
    }

    fn embed_kind(&mut self, kind: SourceKind) -> PlExpr {
        match kind {
            SourceKind::Table(node) => {
                // A table is read as its own wildcard: prqlc has no catalog to
                // enumerate it from, so its columns stay a star.
                self.starred = true;
                node
            }
            SourceKind::Pipeline(other) => {
                let other = *other;
                // An embedded pipeline exposes whatever its own stages
                // projected, so a star crosses the binding only if one was
                // still there to cross.
                self.starred |= other.starred;
                let params = self.values.len();
                let binding_count = other.bindings.len();
                let binding_offset = self.bindings.len();
                self.values.extend(other.values);
                for mut binding in other.bindings {
                    for node in &mut binding {
                        adapter::rebase(node, params, binding_count, binding_offset);
                    }
                    self.bindings.push(binding);
                }
                let mut stages = other.stages;
                for node in &mut stages {
                    adapter::rebase(node, params, binding_count, binding_offset);
                }
                let name = adapter::binding_name(self.bindings.len());
                self.bindings.push(stages);
                adapter::ident(&name)
            }
        }
    }

    /// Append one transform, repointed at the binding a settle left the
    /// relation reading from.
    ///
    /// Every stage the caller writes arrives through here, `staged` or
    /// `ordered_by`, so this is the one place a reference written against a
    /// source the pipeline no longer exposes has to be caught — and the one
    /// place a deduplication that owes a hoist
    /// ([`bare_star_key`](Pipeline::bare_star_key)) learns that something is
    /// following it after all. The hoist happens before the stage is
    /// repointed, so the repointing lands on the binding it just made.
    // [spec:pgorm:req:pipeline.compose+1]
    fn stage(mut self, mut node: PlExpr) -> Self {
        if self.bare_star_key {
            self.settle(&naming::Naming::default(), Some("distinct"));
        }
        if self.combined {
            self.settle(&naming::Naming::default(), None);
        }
        self.settled.requalify(&mut node);
        self.stages.push(node);
        self
    }

    /// Record that `stage` replaced this pipeline's source namespaces,
    /// keeping the *first* offender — the one that did the replacing.
    // [spec:pgorm:sem:pipeline.select-sources+5]
    fn reshaping(mut self, stage: &'static str) -> Self {
        self.reshaped.get_or_insert(stage);
        self
    }

    fn staged(self, nodes: Vec<PlExpr>) -> Self {
        nodes.into_iter().fold(self, Pipeline::stage)
    }

    /// Append a `sort` stage and remember it as the order the relation now
    /// carries, so a deduplication after it can restate the ordering a
    /// `group` would otherwise reset.
    ///
    /// The ordering is read back off the appended stage rather than the
    /// argument, so it is the repointed spelling — the one that still resolves
    /// against whatever binding the relation reads from now.
    // [spec:pgorm:req:pipeline.compose+1]
    fn ordered_by(self, sort: PlExpr) -> Self {
        let mut ordered = self.stage(sort);
        ordered.ordering = ordered.stages.last().cloned();
        ordered
    }

    /// Move the stages accumulated so far into their own binding, leaving
    /// this pipeline reading from it.
    ///
    /// The CTE boundary settles everything the stages left unsettled: the
    /// relation becomes a single freshly exposed input whose columns are its
    /// own projection in its own order, and each stage that had to be
    /// evaluated before whatever comes next now sits in its own query. A
    /// pipeline that is still nothing but its source has nothing to settle.
    ///
    /// What the boundary costs is the sources' own names: behind it the
    /// relation answers under the binding alone. References a later stage
    /// writes against a source are repointed there (`settled`), and
    /// [`select_sources`](Pipeline::select_sources) — which projects *by*
    /// those names and so has nothing to repoint to — is refused by the same
    /// `reshaped` gate the other namespace-replacing stages trip, naming
    /// `owed_to`: the stage the hoist was performed for, which is a
    /// [`distinct`](Pipeline::distinct) that could not compose or a set
    /// operation that would otherwise have reassociated.
    ///
    /// A set operation's own result is settled with no `owed_to`: a set
    /// operation renames nothing, so its sources' references are repointed
    /// at the binding rather than refused.
    // [spec:pgorm:req:pipeline.compose+1]
    // [spec:pgorm:sem:pipeline.select-sources+5]
    pub(super) fn settle(&mut self, naming: &naming::Naming, owed_to: Option<&'static str>) {
        self.combined = false;
        if self.stages.len() <= 1 {
            return;
        }
        if let Some(stage) = owed_to {
            self.reshaped.get_or_insert(stage);
        }
        let stages = std::mem::take(&mut self.stages);
        let name = adapter::binding_name(self.bindings.len());
        self.settled.absorb(&stages, &name, naming);
        self.bindings.push(stages);
        self.stages = vec![adapter::call("from", vec![adapter::ident(&name)])];
        self.deduped = false;
        self.bare_star_key = false;
        self.ranged = false;
        self.columns = Columns::Sourced { introduced: false };
    }

    fn bound<F, T>(&mut self, f: F) -> T
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> T,
    {
        f(&mut Binder::new(&mut self.values))
    }

    /// Keep rows the condition holds for.
    ///
    /// Placement follows position: before an [`aggregate`](Grouped::aggregate)
    /// this becomes `WHERE`, directly after one it becomes `HAVING`, and after
    /// a [`window`](Self::window) the pipeline so far is wrapped in a CTE and
    /// filtered outside it.
    // [spec:pgorm:req:pipeline.surface+4]
    pub fn filter(self, condition: impl Into<Expr<'static>>) -> Self {
        self.stage(adapter::call("filter", vec![condition.into().node]))
    }

    /// Keep rows the condition holds for, with runtime values bound in the
    /// closure.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn filter_with<F>(mut self, f: F) -> Self
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> Expr<'brand>,
    {
        let node = self.bound(|binder| f(binder).node);
        self.stage(adapter::call("filter", vec![node]))
    }

    /// Add computed columns, keeping the existing ones.
    // [spec:pgorm:req:pipeline.surface+4]
    pub fn derive(self, columns: impl ExprList<'static>) -> Self {
        self.derive_nodes(nodes_of(columns))
    }

    /// Add computed columns, with runtime values bound in the closure.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn derive_with<F, const N: usize>(mut self, f: F) -> Self
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> [Expr<'brand>; N],
    {
        let nodes = self.bound_nodes(f);
        self.derive_nodes(nodes)
    }

    fn derive_nodes(self, nodes: Vec<PlExpr>) -> Self {
        let nodes = window::written_aggregates(nodes, IMPLICIT_WINDOW);
        // The shape is recorded off the staged pipeline: appending may have
        // discharged a hoist, and what the new names trail is then the
        // binding's columns rather than the ones they were written beside.
        let mut derived = self.stage(adapter::call("derive", vec![adapter::tuple(nodes)]));
        derived.columns = derived.columns.with_introduced();
        derived
    }

    /// Replace the projection with exactly these columns.
    // [spec:pgorm:req:pipeline.surface+4]
    pub fn select(self, columns: impl ExprList<'static>) -> Self {
        self.select_nodes(nodes_of(columns))
    }

    /// Replace the projection, with runtime values bound in the closure.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn select_with<F, const N: usize>(mut self, f: F) -> Self
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> [Expr<'brand>; N],
    {
        let nodes = self.bound_nodes(f);
        self.select_nodes(nodes)
    }

    pub(super) fn select_nodes(mut self, mut nodes: Vec<PlExpr>) -> Self {
        // Read the shape off the names the relation actually answers to: a
        // list written against two sources a settle has since collapsed into
        // one binding is one source's columns, not two.
        for node in &mut nodes {
            self.settled.requalify(node);
        }
        let nodes = window::written_aggregates(nodes, IMPLICIT_WINDOW);
        let columns = Columns::of(&nodes);
        // A projection replaces the relation's columns, so whatever wildcard
        // the sources carried is gone and the listed items are all there is.
        self.starred = false;
        // A projection may drop the very columns an earlier sort ordered by,
        // so the ordering it described no longer names anything this relation
        // can be sorted on again.
        self.ordering = None;
        let mut selected = self
            .stage(adapter::call("select", vec![adapter::tuple(nodes)]))
            .reshaping("select");
        selected.columns = columns;
        selected
    }

    /// Group rows by these keys; the aggregates follow.
    ///
    /// ```
    /// # use pgorm::pipeline::{ExprOps, Pipeline, sum};
    /// # use pgorm::pgorm_query::alias;
    /// # use pgorm::tests_cfg::cake::{self, Column as C};
    /// let spent = alias("spent");
    /// let (sql, _) = Pipeline::from(cake::Entity)
    ///     .group(C::Name)
    ///     .aggregate(sum(C::Id).as_(spent))
    ///     .filter(spent.gt(2))
    ///     .into_sql()?;
    /// assert!(sql.contains("GROUP BY name"));
    /// assert!(sql.contains("HAVING"));
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    // [spec:pgorm:req:pipeline.surface+4]
    pub fn group(self, keys: impl ExprList<'static>) -> Grouped {
        let keys = nodes_of(keys);
        Grouped {
            pipeline: self,
            keys,
        }
    }

    /// Group rows by keys computed with runtime values bound in the closure.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn group_with<F, const N: usize>(mut self, f: F) -> Grouped
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> [Expr<'brand>; N],
    {
        let keys = self.bound_nodes(f);
        Grouped {
            pipeline: self,
            keys,
        }
    }

    /// Derive columns over a window: what to compute, and what to compute it
    /// over ([`by`](crate::pipeline::by), [`sort_by`](crate::pipeline::sort_by),
    /// [`over`](crate::pipeline::over)).
    ///
    /// With a partition this compiles to `PARTITION BY` under a `group`
    /// stage; without one the window spans the whole relation.
    // [spec:pgorm:req:pipeline.surface+4]
    pub fn window(self, columns: impl ExprList<'static>, over: Over) -> Self {
        self.window_nodes(nodes_of(columns), over)
    }

    /// Derive columns over a window, with runtime values bound in the
    /// closure.
    ///
    /// The window spec comes first here so that the closure stays last, as
    /// it does in every `_with` transform.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn window_with<F, const N: usize>(mut self, over: Over, f: F) -> Self
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> [Expr<'brand>; N],
    {
        let nodes = self.bound_nodes(f);
        self.window_nodes(nodes, over)
    }

    fn window_nodes(self, nodes: Vec<PlExpr>, over: Over) -> Self {
        // The ordering the relation already carries is the ordering prqlc
        // reads into an unpartitioned window that states none of its own, so
        // a window that writes its own `OVER` clause has to be told it.
        let mut carried = self.ordering.clone();
        let inherited = carried
            .as_mut()
            .and_then(adapter::tuple_items_mut)
            .map(std::mem::take)
            .unwrap_or_default();
        let staged = over.wrap(nodes, inherited);
        // An unpartitioned window's own ordering is a real pipeline stage, so
        // it orders the output too, and a deduplication after it inherits
        // that order exactly as it inherits a `sort`'s. A partitioned one
        // nests its ordering inside the `group` instead, where it orders only
        // the window.
        let ordering = staged
            .iter()
            .find(|node| adapter::stage_verb(node) == Some("sort"))
            .cloned();
        // As in `derive_nodes`, the shape is read off the staged pipeline so
        // that a hoist discharged on the way in is what the new names trail.
        let mut windowed = self.staged(staged);
        windowed.columns = windowed.columns.with_introduced();
        if let Some(sort) = ordering {
            windowed.ordering = Some(sort);
        }
        windowed
    }

    /// Sort by these keys ([`desc`](super::ExprOps::desc) marks one
    /// descending).
    // [spec:pgorm:req:pipeline.surface+4]
    pub fn sort(self, keys: impl ExprList<'static>) -> Self {
        self.ordered_by(adapter::call("sort", vec![adapter::tuple(nodes_of(keys))]))
    }

    /// Sort by keys computed with runtime values bound in the closure.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn sort_with<F, const N: usize>(mut self, f: F) -> Self
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> [Expr<'brand>; N],
    {
        let nodes = self.bound_nodes(f);
        self.ordered_by(adapter::call("sort", vec![adapter::tuple(nodes)]))
    }

    /// Keep the first `rows` rows (`LIMIT`).
    ///
    /// The count is a value, not an expression: PRQL rejects a parameterized
    /// `take`, so the signature takes the only form that compiles.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn take(self, rows: i64) -> Self {
        let mut taken = self.stage(adapter::call("take", vec![adapter::lit_int(rows)]));
        taken.ranged = true;
        taken
    }

    /// Keep an inclusive 1-based row range (`LIMIT`/`OFFSET`).
    // [spec:pgorm:req:pipeline.params+4]
    pub fn take_range(self, rows: RangeInclusive<i64>) -> Self {
        let mut taken = self.stage(adapter::call(
            "take",
            vec![adapter::int_range(Some(*rows.start()), Some(*rows.end()))],
        ));
        taken.ranged = true;
        taken
    }

    /// Join another relation on an explicit condition.
    ///
    /// Both sides of the condition are columns, and an entity column carries
    /// its table, so the condition is qualified by construction. When the
    /// joined relation is a pipeline, its columns are referred to by their
    /// own names — unqualified — or by [`that`](super::that) where a name
    /// exists on both sides. To join a relation to itself, give the operand
    /// a name of its own with [`named`](IntoSource::named).
    // [spec:pgorm:req:pipeline.surface+4]
    // [spec:pgorm:sem:pipeline.self-join+1]
    pub fn join(
        self,
        side: JoinSide,
        relation: impl IntoSource,
        on: impl Into<Expr<'static>>,
    ) -> Self {
        self.join_node(side, relation, on.into().node)
    }

    /// Join another relation, with runtime values bound in the closure.
    // [spec:pgorm:req:pipeline.params+4]
    pub fn join_with<F>(mut self, side: JoinSide, relation: impl IntoSource, on: F) -> Self
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> Expr<'brand>,
    {
        let node = self.bound(|binder| on(binder).node);
        self.join_node(side, relation, node)
    }

    fn join_node(mut self, side: JoinSide, relation: impl IntoSource, condition: PlExpr) -> Self {
        let reference = self.embed(relation.into_source());
        // The joined relation is in scope from here under its own name, which
        // a settle may have shadowed earlier — so the condition's references
        // to it are the caller's, not a settled binding's, and must survive
        // the repointing the stage is about to go through.
        self.settled.rejoined(&reference);
        let mut joined = self.stage(adapter::call_named(
            "join",
            vec![reference, condition],
            vec![("side", adapter::ident(side.keyword()))],
        ));
        // The joined relation's columns follow this one's, which is the order
        // `this` expands them in — but only while no introduced name is
        // already competing with an input for the same position. Read after
        // staging, so a hoist discharged on the way in is the left side here.
        joined.columns = match joined.columns {
            Columns::Sourced { introduced: false } => Columns::Sourced { introduced: false },
            _ => Columns::Regrouped,
        };
        joined
    }

    /// Concatenate another relation's rows after this one's: PRQL's
    /// `append`, SQL's `UNION ALL`.
    ///
    /// Columns correspond by position, and prqlc refuses the append at
    /// [`into_sql`](Pipeline::into_sql) when it can see the two column
    /// counts differ. Follow with [`distinct`](Pipeline::distinct) for
    /// `UNION DISTINCT`.
    ///
    /// A stage after a set operation reads the whole combined relation: the
    /// operation is settled into a binding first, which is what keeps a
    /// projection from landing on the first operand alone.
    ///
    /// ```
    /// use pgorm::pipeline::{Pipeline, alias};
    ///
    /// let (sql, _) = Pipeline::from(alias("cake"))
    ///     .append(alias("archived_cake"))
    ///     .select(alias("name"))
    ///     .into_sql()?;
    /// assert_eq!(
    ///     sql,
    ///     "WITH table_0 AS (SELECT * FROM cake UNION ALL SELECT * FROM archived_cake) \
    ///      SELECT name FROM table_0"
    /// );
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    // [spec:pgorm:req:pipeline.compose+1]
    pub fn append(self, other: impl IntoSource) -> Self {
        self.set_op("append", other)
    }

    /// Keep only rows that also appear in `other`: PRQL's `intersect`,
    /// SQL's `INTERSECT ALL`.
    ///
    /// The stages after it read the result as they do after
    /// [`remove`](Self::remove).
    ///
    /// This is the one set operation SQL binds tightly, so written after an
    /// [`append`](Self::append) or a [`remove`](Self::remove) it takes the
    /// relation so far as a binding rather than as another operator in the
    /// same chain — `(a UNION ALL b) INTERSECT ALL c`, which is what the
    /// pipeline says, and not the `a UNION ALL (b INTERSECT ALL c)` that
    /// precedence would otherwise make of it.
    // [spec:pgorm:req:pipeline.compose+1]
    pub fn intersect(self, other: impl IntoSource) -> Self {
        self.set_op("intersect", other).reshaping("intersect")
    }

    /// Drop rows that appear in `other`: PRQL's `remove`, SQL's
    /// `EXCEPT ALL` — each row in `other` cancels one matching row here,
    /// not all of them.
    ///
    /// The stages after it read the result from a binding, as after
    /// [`append`](Self::append): a column is named bare (an alias token) or
    /// against the source it came from, `col(INVOICE, ..)` or an entity
    /// column, which is repointed at the binding. Over operands whose
    /// columns are wildcards prqlc expands neither, so only a stage that
    /// reads no column composes there (`take`, `distinct`); a pipeline that
    /// reads the result projects its operands first.
    // [spec:pgorm:req:pipeline.compose+1]
    pub fn remove(self, other: impl IntoSource) -> Self {
        self.set_op("remove", other).reshaping("remove")
    }

    fn set_op(mut self, op: &'static str, other: impl IntoSource) -> Self {
        // Two things force the pending stages into a binding of their own. A
        // deduplicating `group` the set operation would otherwise be taken off
        // directly has to move, so the arity settles at the CTE boundary; and
        // a set operation already pending that SQL binds *looser* than this
        // one has to move too, because a flat chain of the two is reassociated
        // by precedence into a relation the pipeline never wrote (`sets`).
        // Either way the binding is the bracket, and nothing was renamed on
        // the way: a deduplication that owes the hoist made the projection
        // addressable already, and a set operation renames nothing.
        let owed_to = if self.deduped {
            Some("distinct")
        } else if sets::reassociates(&self.stages, op) {
            Some(op)
        } else {
            None
        };
        if let Some(stage) = owed_to {
            self.settle(&naming::Naming::default(), Some(stage));
        }
        // The combined relation is not the one the ordering described, and
        // for `intersect` and `remove` it does not even answer to the same
        // names.
        self.ordering = None;
        let reference = self.embed(other.into_source());
        self.combined = false;
        let mut combined = self.stage(adapter::call(op, vec![reference]));
        combined.combined = true;
        combined
    }

    /// Keep one copy of each distinct row: PRQL's `group this (take 1)`,
    /// rendered `SELECT DISTINCT` — or folded into `UNION DISTINCT` when it
    /// directly follows [`append`](Pipeline::append).
    ///
    /// The deduplicating group does not compose with every stage that can
    /// stand in front of it, so the pipeline is settled into its own binding
    /// first where it would not: prqlc resolves the group's `this` against a
    /// namespace rather than the relation's column list, and renders a row
    /// range standing in front of the group *after* the deduplication rather
    /// than before it. Reading from a binding leaves the group a single
    /// freshly exposed relation, whose columns are that binding's projection
    /// in its own order with nothing in front of them.
    ///
    /// Settling costs the sources their own names: behind a binding the
    /// relation answers under that binding alone. A reference a later stage
    /// writes against a source — `col(source, column)`, an entity column —
    /// is repointed there as it is appended, so composing on is unaffected;
    /// [`select_sources`](Pipeline::select_sources), which projects *by* those
    /// names and so has nothing to repoint to, is refused instead.
    ///
    /// Because the key is a namespace, it reaches only the columns that have
    /// a name of their own: an unnamed expression and the loser of a name
    /// collision are both absent from it — and so from the result, which is
    /// what the key projects. The projection is made addressable first, which
    /// gives those columns names and settles the relation behind them.
    ///
    /// A `sort` in front of the deduplication is restated behind it. PRQL's
    /// `group` resets the order, so an ordering written before one is undone
    /// rather than applied; writing it again afterwards is the
    /// `distinct` then `sort` shape, which renders the `DISTINCT` in a
    /// binding and the `ORDER BY` outside it.
    ///
    /// One column has no name to be made addressable by: the wildcard a
    /// relation whose columns no projection has yet replaced still carries.
    /// The key holds it too, and prqlc can spell it there only while it
    /// qualifies — `cake.*` is a whole-row reference, a bare `*` is not an
    /// expression PostgreSQL has. So a deduplication over such a relation is
    /// hoisted into a binding of its own as soon as another stage follows it,
    /// which leaves the key equal to the relation's frame and the rendering
    /// plain `SELECT DISTINCT *`. Nothing following means nothing owed, so a
    /// deduplication that ends the pipeline still renders as the one query it
    /// always was:
    ///
    /// ```
    /// # use pgorm::pipeline::Pipeline;
    /// # use pgorm::tests_cfg::cake::{self, Column as C};
    /// let (alone, _) = Pipeline::from(cake::Entity).distinct().into_sql()?;
    /// assert_eq!(alone, "SELECT DISTINCT * FROM cake");
    ///
    /// let (projected, _) = Pipeline::from(cake::Entity)
    ///     .distinct()
    ///     .select(C::Name)
    ///     .into_sql()?;
    /// assert_eq!(
    ///     projected,
    ///     "WITH table_0 AS (SELECT DISTINCT * FROM cake) SELECT name FROM table_0"
    /// );
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    ///
    /// The two are different questions and the hoist is what keeps them
    /// apart: deduplicating whole rows and *then* projecting is not the same
    /// relation as projecting and then deduplicating, which is what
    /// `select` before `distinct` says.
    ///
    /// ```
    /// # use pgorm::pipeline::{ExprOps, Pipeline, alias, col};
    /// # let accounts = alias("accounts");
    /// let (sql, _) = Pipeline::from(accounts)
    ///     .select((col(accounts, alias("id")), col(accounts, alias("name"))))
    ///     .sort(col(accounts, alias("id")))
    ///     .distinct()
    ///     .into_sql()?;
    /// assert_eq!(
    ///     sql,
    ///     "WITH table_0 AS (SELECT DISTINCT id, name FROM accounts) \
    ///      SELECT id, name FROM table_0 ORDER BY id"
    /// );
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    // [spec:pgorm:req:pipeline.compose+1]
    pub fn distinct(mut self) -> Self {
        let naming = naming::disambiguate(&mut self.stages);
        let ordering = self.ordering.take();
        let settled = naming.renamed || self.ranged || !self.columns.ordered();
        if settled {
            self.settle(&naming, Some("distinct"));
        }
        // Whether *this* deduplication's key will hold a star with no legal
        // spelling, read after any hoist above, because that is the relation
        // the group will key on.
        let bare_key = naming::bare_star_key(&self.stages, self.starred);
        // Deduplicating again adds nothing to the frame, so prqlc folds the
        // two into one `DISTINCT` and a hoist an earlier one owes is not owed
        // to this stage.
        self.bare_star_key = false;
        self.deduped = true;
        // Deduplicating a set operation's result folds into the operation,
        // so the settle it owes is owed to the stage after this one.
        let combined = std::mem::take(&mut self.combined);
        let mut deduplicated = self.stage(adapter::call(
            "group",
            vec![
                adapter::ident("this"),
                adapter::call("take", vec![adapter::lit_int(1)]),
            ],
        ));
        // Behind a binding the relation answers to bare names alone, so a key
        // written against the source it came from has to be repointed — and
        // one there is no name to repoint is left unrestated, which is the
        // order the pipeline had before.
        let restated = match ordering {
            Some(sort) if settled => naming::rebound_sort(sort, &naming),
            unsettled => unsettled,
        };
        deduplicated.combined = combined;
        deduplicated.bare_star_key = bare_key;
        // The restated ordering is a stage like any other, so it discharges
        // the hoist: the binding the group lands in is then the one prqlc
        // would have minted for the `ORDER BY` anyway, rather than a second
        // one wrapping it.
        match restated {
            Some(sort) => deduplicated.ordered_by(sort),
            None => deduplicated,
        }
    }

    fn bound_nodes<F, const N: usize>(&mut self, f: F) -> Vec<PlExpr>
    where
        F: for<'brand> FnOnce(&mut Binder<'brand>) -> [Expr<'brand>; N],
    {
        self.bound(|binder| f(binder).into_iter().map(|expr| expr.node).collect())
    }
}
