//! The N-ary relational read: a root entity plus a typed list of joined,
//! decoded sources.
//!
//! [`SelectGraph<E, S>`] is one shape at every arity where the pair surface
//! had a bespoke type per pair. The declaration is the whole contract:
//!
//! - the *slot kind is the join type is the decode shape* — [`Opt<F>`] is a
//!   `LEFT JOIN` decoded through the absence witness as `Option<F::Model>`,
//!   [`Req<F>`] is an `INNER JOIN` decoded as a bare `F::Model` — so
//!   "LEFT-joined but decoded as required" is unrepresentable;
//! - the projection is *generated only*, by the one writer
//!   ([`project_source`], prefix `s{i}_`, the `select_as` enum-cast
//!   discipline included), and the builder implements no [`QuerySelect`], so
//!   a hand-edited projection that the decode does not expect cannot be
//!   written;
//! - the decode target is `(E::Model, S1::Out, …, Sn::Out)`, computed from
//!   the declared slot tuple, so a mismatch is a type error rather than a
//!   run-time surprise.
//!
//! [`QuerySelect`]: crate::QuerySelect

use core::marker::PhantomData;
use std::fmt;

use pgorm_query::{
    Condition, Expr, FromItem, IntoName, JoinType, Name, NamedTable, SelectExpr, SelectStatement,
    TableName, TypeName,
};

use super::helper::{alias_apart, join_condition};
use crate::executor::BoundaryCast;
use crate::executor::result_name::result_column_name;
use crate::{
    ColumnTrait, EntityTrait, Error, FromQueryResult, Iterable, PrimaryKeyToColumn, QueryFilter,
    QueryOrder, QueryResult, QueryTrait, Related, RelationDef, StaticName,
};

/// The closure shape a call-site `ON` predicate takes: the join's left and
/// right identifiers in, a [`Condition`] out — the same shape
/// [`RelationDef::on_condition`] takes.
type OnCondition = Box<dyn Fn(Name, Name) -> Condition + Send + Sync>;

mod sealed {
    /// [`Slot`](super::Slot) is sealed: the join type, the decode and the
    /// output shape are one declaration, and an outside implementor could
    /// only make the three disagree.
    pub trait Sealed {}
}

/// A joined source that must match: `INNER JOIN`, decoded as a bare `Model`.
///
/// The type states that the join cannot miss, so there is no `Option` to
/// unwrap for a row the join guarantees.
// [spec:pgorm:sem:query.graph.slots+2]
pub struct Req<F>(PhantomData<F>);

/// A joined source that may be absent: `LEFT JOIN`, decoded through the
/// absence witness ([`FromQueryResult::from_query_result_optional`]) as
/// `Option<Model>`.
// [spec:pgorm:sem:query.graph.slots+2]
pub struct Opt<F>(PhantomData<F>);

impl<F> fmt::Debug for Req<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Req")
    }
}

impl<F> fmt::Debug for Opt<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Opt")
    }
}

impl<F: EntityTrait> sealed::Sealed for Req<F> {}
impl<F: EntityTrait> sealed::Sealed for Opt<F> {}

/// The row range of a graph that neither [`limit`](SelectGraph::limit) nor
/// [`offset`](SelectGraph::offset) has cut: every row the query matches.
///
/// Only an unsliced graph reaches the reads that decide the row range
/// themselves — the grouped read, the paginator and the cursor.
// [spec:pgorm:sem:query.graph.slice]
#[derive(Debug)]
pub enum Unsliced {}

/// The row range of a graph that [`limit`](SelectGraph::limit) or
/// [`offset`](SelectGraph::offset) has cut: a window of rows of the join
/// product, not of roots.
// [spec:pgorm:sem:query.graph.slice]
#[derive(Debug)]
pub enum Sliced {}

/// One declared source of a [`SelectGraph`]: its entity, its output shape,
/// the join type that shape implies, and the decode that reads it.
///
/// The trait is sealed — [`Req<F>`] and [`Opt<F>`] are the only slots — so
/// the four cannot be recombined into a pairing the graph does not mean.
// [spec:pgorm:sem:query.graph.slots+2]
pub trait Slot: sealed::Sealed {
    /// The entity behind the slot.
    type Entity: EntityTrait;

    /// What one row contributes for this slot: `F::Model` for [`Req`],
    /// `Option<F::Model>` for [`Opt`].
    type Out;

    /// The join type the slot's shape implies.
    const JOIN: JoinType;

    /// Decode this slot's columns out of `res`, under the prefix `pre` the
    /// writer aliased them with.
    fn decode(res: &QueryResult, pre: &str) -> Result<Self::Out, Error>;
}

// [spec:pgorm:sem:query.graph.slots+2]
impl<F: EntityTrait> Slot for Req<F> {
    type Entity = F;
    type Out = F::Model;
    const JOIN: JoinType = JoinType::InnerJoin;

    // [spec:pgorm:sem:query.graph.decode+1]
    fn decode(res: &QueryResult, pre: &str) -> Result<Self::Out, Error> {
        F::Model::from_query_result(res, pre)
    }
}

// [spec:pgorm:sem:query.graph.slots+2]
impl<F: EntityTrait> Slot for Opt<F> {
    type Entity = F;
    type Out = Option<F::Model>;
    const JOIN: JoinType = JoinType::LeftJoin;

    // [spec:pgorm:sem:query.graph.decode+1]
    fn decode(res: &QueryResult, pre: &str) -> Result<Self::Out, Error> {
        F::Model::from_query_result_optional(res, pre)
    }
}

/// One secondary order entry a decoded source contributes to a cursor's sort
/// key: a primary-key column, the identifier it is qualified with, and the
/// casts a boundary compares it under.
///
/// The casts are captured here, where the typed column is in hand: the
/// cursor's keyset is identifiers only, and a key whose domain is over an
/// enum has no comparison operator unless it and its value are cast.
// [spec:pgorm:sem:query.graph.cursor+4]
#[derive(Debug, Clone)]
pub struct Tiebreak {
    pub(crate) table: Name,
    pub(crate) column: Name,
    pub(crate) cast: BoundaryCast,
}

/// The secondary order entries one decoded source contributes to a cursor's
/// sort key: one unary entry per primary-key column.
pub(crate) type Tiebreaks = Vec<Tiebreak>;

/// One decoded source's primary key as cursor tiebreaks, qualified with the
/// source's *effective* identifier — its bound alias when it has one — so a
/// tiebreak names the same table the projection and the `ON` clause do.
// [spec:pgorm:sem:query.graph.cursor+4]
pub(crate) fn qualified_pk_tiebreaks<F: EntityTrait>(qualifier: &Name) -> Tiebreaks {
    <F::PrimaryKey as Iterable>::iter()
        .map(|pk| {
            let column = pk.into_column();
            Tiebreak {
                table: Name::clone(qualifier),
                column: Name::new(column),
                cast: BoundaryCast::of(column),
            }
        })
        .collect()
}

/// The declared slot tuple read as a list: what a cursor needs to install one
/// tiebreak per decoded slot without a call site naming a column.
///
/// The declaration that fixed the joins fixes the tiebreak set, so the trait
/// is sealed for the reason [`Slot`] is — an outside implementor could only
/// make the two disagree. It is implemented for `()` and for every slot tuple
/// the graph can declare.
// [spec:pgorm:sem:query.graph.cursor+4]
pub trait Slots: sealed::Sealed {
    /// The primary-key tiebreaks of every declared slot except `skip`, in
    /// declaration order.
    ///
    /// `qualifiers` holds the slots' effective identifiers in declaration
    /// order — the root's is not among them — and slots are numbered from 1
    /// as the projection prefixes are (`s1_` is the first slot), so
    /// `skip == 0` skips nothing.
    fn tiebreaks(qualifiers: &[Name], skip: usize) -> Tiebreaks;
}

/// The slot declared at position `I`, counted from 1 as the projection
/// prefixes are: `s1_` is the first slot, the root being source 0.
///
/// This is how [`cursor_by_on`](crate::SelectGraph::cursor_by_on) names a
/// slot — positionally, at compile time. A position no slot occupies has no
/// implementation, so asking for it is a compile error rather than a silently
/// mis-qualified column.
// [spec:pgorm:sem:query.graph.cursor+4]
pub trait SlotAt<const I: usize>: Slots {
    /// The slot at that position, whose entity types the order columns.
    type Slot: Slot;
}

/// A slotless graph declares no tiebreaks; its cursor is a single-table one.
// [spec:pgorm:sem:query.graph.cursor+4]
impl Slots for () {
    fn tiebreaks(_qualifiers: &[Name], _skip: usize) -> Tiebreaks {
        Tiebreaks::new()
    }
}

impl sealed::Sealed for () {}

/// Generate the `SlotAt` impls of one slot-tuple arity, one position per
/// recursion, so each impl sees the whole tuple and the single slot it picks.
macro_rules! slot_at {
    ( ( $( $all:ident ),+ ) ; ) => {};
    ( ( $( $all:ident ),+ ) ; $s:ident @ $i:literal $( , $rest:ident @ $ri:literal )* ) => {
        // [spec:pgorm:sem:query.graph.cursor+4]
        impl< $( $all: Slot ),+ > SlotAt<$i> for ( $( $all, )+ ) {
            type Slot = $s;
        }

        slot_at!( ( $( $all ),+ ) ; $( $rest @ $ri ),* );
    };
}

/// Generate the slot-list machinery for one slot-tuple arity: the seal, the
/// tiebreak list, and one `SlotAt` impl per declared position.
macro_rules! slots {
    ( $( $s:ident @ $i:literal ),+ ) => {
        impl< $( $s: Slot ),+ > sealed::Sealed for ( $( $s, )+ ) {}

        // [spec:pgorm:sem:query.graph.cursor+4]
        impl< $( $s: Slot ),+ > Slots for ( $( $s, )+ ) {
            fn tiebreaks(qualifiers: &[Name], skip: usize) -> Tiebreaks {
                let sources: &[fn(&Name) -> Tiebreaks] =
                    &[ $( qualified_pk_tiebreaks::<<$s as Slot>::Entity> ),+ ];

                sources
                    .iter()
                    .zip(qualifiers)
                    .enumerate()
                    .filter(|(index, _)| index + 1 != skip)
                    .flat_map(|(_, (tiebreaks, qualifier))| tiebreaks(qualifier))
                    .collect()
            }
        }

        slot_at!( ( $( $s ),+ ) ; $( $s @ $i ),+ );
    };
}

slots!(S1 @ 1);
slots!(S1 @ 1, S2 @ 2);
slots!(S1 @ 1, S2 @ 2, S3 @ 3);
slots!(S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4);
slots!(S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4, S5 @ 5);
slots!(S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4, S5 @ 5, S6 @ 6);

/// THE prefix scheme: the prefix of the `index`-th decoded source,
/// `s{index}_`, under which [`result_column_name`] composes every alias of
/// that source, bounded to PostgreSQL's 63-byte identifier limit — the same
/// composition every prefixed read looks names up by.
///
/// Both projection writers — `project_source` into a [`SelectStatement`]
/// and the pipeline's `select_sources` terminal into prqlc's PL AST — mint
/// their aliases under it, and the per-prefix decodes read exactly these
/// names, so the scheme is one code path rather than a convention every site
/// repeats. A binding decoding a prefixed source of its own reads by it too.
// [spec:pgorm:sem:query.graph.writer+4]
pub fn source_prefix(index: usize) -> String {
    format!("s{index}_")
}

/// THE read-cast discipline, derived from the column's own `select_as`
/// answer: the type a projected column is `CAST` to on selection, or `None`
/// to project it untouched.
///
/// The pipeline's writer emits PRQL nodes rather than
/// [`SimpleExpr`](pgorm_query::SimpleExpr), so it cannot splice the method's
/// answer in directly. Instead the answer is probed with the bare column and
/// the cast target read back off its shape. A cast has exactly one shape, so
/// the probe recognises one — the enum default (`text` / `text[]`) and a
/// `#[pgorm(select_as = "…")]` override's type expression alike, both riding
/// in the `AsEnum`'s `TypeName` — and the two writers cannot disagree about
/// a column's read cast. A shape that is not a cast — a hand-written
/// override projecting something else — projects untouched, exactly as it
/// stands outside the `SelectStatement` writer's guarantees too.
///
/// The target comes back as the `TypeName` itself rather than as text, so
/// the pipeline renders it through the same part policy
/// ([`TypeName::to_sql_string`]) the `SelectStatement` writer does: a name is
/// quoted where it has to be, and only a `TypeName::raw` type expression —
/// program text — is written as it stands.
// [spec:pgorm:sem:query.graph.writer+4]
// [spec:pgorm:sem:pipeline.select-sources+5]
// [spec:pgorm:req:sql.ast.cast-shape+2]
pub(crate) fn source_read_cast<C: ColumnTrait>(col: &C) -> Option<TypeName> {
    use pgorm_query::SimpleExpr;
    match col.select_as(Expr::col(Name::new(*col))) {
        SimpleExpr::AsEnum(type_name, _) => Some(*type_name),
        _ => None,
    }
}

/// THE one projection writer: every decoded source of a graph — and of the
/// pipeline's source-select terminal — passes through here.
///
/// Appends, for every variant of `F`'s `Column` in iteration order,
/// `col.select_as(Expr::col((qualifier, col)))` aliased as
/// `s{index}_{col}` ([`source_prefix`]) — the same enum-cast
/// discipline as `Select<E>`'s default list, with the alias being the plain
/// SQL column name under the prefix, whatever the cast wrapped.
///
/// `qualifier` is the source's *effective* identifier: the bound alias when
/// the source was declared under one, otherwise its bare table — the same
/// identifier the `ON` clause constrains against, so the projection and the
/// join cannot name one source two ways.
// [spec:pgorm:sem:query.graph.writer+4]
pub(crate) fn project_source<F: EntityTrait>(
    query: &mut SelectStatement,
    qualifier: Name,
    index: usize,
) {
    let prefix = source_prefix(index);
    for col in <F::Column as Iterable>::iter() {
        let alias = result_column_name(&prefix, col.as_str());
        let expr = Expr::col((Name::clone(&qualifier), col.into_name()));
        query.expr(SelectExpr::new_as(
            col.select_as(expr),
            Name::runtime(alias),
        ));
    }
}

/// A relational read declared as a root entity plus joined, decoded sources.
///
/// `S` is the tuple of declared slots, in join order; each edge method
/// appends its slot by returning `SelectGraph<E, (…, NewSlot)>`, so the
/// declared tuple, the emitted joins and the decoded row type are one fact
/// stated once. Construct with [`EntityTrait::graph`](crate::EntityTrait::graph),
/// [`SelectGraph::new`] or [`Default`]; the root's columns are projected
/// under `s0_` from the moment the value exists.
///
/// ```
/// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit}};
///
/// assert_eq!(
///     cake::Entity::graph()
///         .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
///         .as_query()
///         .to_string(),
///     [
///         r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
///         r#""fruit"."id" AS "s1_id", "fruit"."name" AS "s1_name", "fruit"."cake_id" AS "s1_cake_id""#,
///         r#"FROM "cake""#,
///         r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// `R` records whether [`limit`](Self::limit) or [`offset`](Self::offset)
/// has cut the read to a window of rows: [`Unsliced`] until one is called,
/// [`Sliced`] after.
///
/// The projection is generated, never edited: the graph does not implement
/// [`QuerySelect`](crate::QuerySelect), so there is no `column`,
/// `column_as`, `expr`, `select` or `select_only` to disagree with the
/// decode.
///
/// ```compile_fail,E0599
/// use pgorm::{entity::*, query::*, tests_cfg::cake};
///
/// // The graph's projection is generated, never edited.
/// let _ = cake::Entity::graph().column(cake::Column::Name);
/// ```
///
/// Neither can the decode be told a shape the declaration does not produce:
/// the row type is computed from `S`.
///
/// ```compile_fail,E0308
/// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit, vendor}};
/// use pgorm::query::{Opt, SelectGraph};
///
/// // The graph joined `fruit`, so its slot list is `(Opt<fruit::Entity>,)`;
/// // ascribing a vendor slot is a type error, not a runtime decode surprise.
/// let _: SelectGraph<cake::Entity, (Opt<vendor::Entity>,)> = cake::Entity::graph()
///     .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def());
/// ```
// [spec:pgorm:def:query.graph+2]
#[doc(alias("SelectTwo", "SelectTwoMany"))]
pub struct SelectGraph<E: EntityTrait, S = (), R = Unsliced> {
    pub(crate) query: SelectStatement,
    /// The decoded sources' effective identifiers, in declaration order: the
    /// root at index 0, each slot at its own. It is the writer's prefix index
    /// and the cursor's tiebreak qualifier, held once so the two cannot
    /// disagree about what a source is called.
    pub(crate) qualifiers: Vec<Name>,
    /// Every table the graph names, the root and each edge's joined side, in
    /// join order.
    pub(crate) visits: Vec<Visit>,
    pub(crate) marker: PhantomData<(E, S, R)>,
}

/// A table a graph's statement names: the identifier its columns are
/// qualified by, the table, and whether the graph aliased it apart from
/// another table of its bare name, which makes the identifier the one an
/// edge leaving from the table leaves from.
// [spec:pgorm:req:query.graph.aliases+2]
#[derive(Clone, Debug)]
pub(crate) struct Visit {
    qualifier: Name,
    table: Option<TableName>,
    apart: bool,
}

/// A table as a [`Visit`] compares it: its schema, where it has one, and its
/// bare name, whatever types spell them.
fn identity(table: &TableName) -> (Option<String>, String) {
    (
        table.schema().map(|schema| schema.to_string()),
        table.table().to_string(),
    )
}

impl<E: EntityTrait, S, R> fmt::Debug for SelectGraph<E, S, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectGraph")
            .field("query", &self.query)
            .field("qualifiers", &self.qualifiers)
            .field("visits", &self.visits)
            .finish()
    }
}

impl<E: EntityTrait, S, R> Clone for SelectGraph<E, S, R> {
    fn clone(&self) -> Self {
        SelectGraph {
            query: self.query.clone(),
            qualifiers: self.qualifiers.clone(),
            visits: self.visits.clone(),
            marker: PhantomData,
        }
    }
}

// [spec:pgorm:def:query.graph+2]
impl<E: EntityTrait> SelectGraph<E, ()> {
    /// Start a graph at its root entity, selected `FROM` its table and
    /// projected under the `s0_` prefix.
    ///
    /// The same construction [`EntityTrait::graph`](crate::EntityTrait::graph)
    /// and [`Default`] perform.
    pub fn new() -> Self {
        E::graph_in(E::default())
    }

    /// The graph over `query`, whose `FROM` reads `table`, one of `E`'s, its
    /// root projected under the `s0_` prefix.
    // [spec:pgorm:def:query.graph+2]
    pub(crate) fn rooted(query: SelectStatement, table: TableName) -> Self {
        let mut graph = SelectGraph {
            query,
            qualifiers: Vec::new(),
            visits: vec![Visit {
                qualifier: Name::new(E::default()),
                table: Some(table),
                apart: false,
            }],
            marker: PhantomData,
        };
        graph.project::<E>(Name::new(E::default()));
        graph
    }
}

// [spec:pgorm:def:query.graph+2]
impl<E: EntityTrait> Default for SelectGraph<E, ()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: EntityTrait, S, R> SelectGraph<E, S, R> {
    /// Project one decoded source under the next prefix, and record the
    /// identifier that prefix belongs to.
    // [spec:pgorm:sem:query.graph.writer+4]
    pub(crate) fn project<F: EntityTrait>(&mut self, qualifier: Name) {
        project_source::<F>(
            &mut self.query,
            Name::clone(&qualifier),
            self.qualifiers.len(),
        );
        self.qualifiers.push(qualifier);
    }

    /// The declared slots' effective identifiers, in declaration order — the
    /// root's excluded, so slot `n` sits at index `n - 1`.
    // [spec:pgorm:sem:query.graph.cursor+4]
    pub(crate) fn slot_qualifiers(&self) -> &[Name] {
        self.qualifiers.get(1..).unwrap_or_default()
    }

    /// The effective identifier of one decoded source: the root at 0, each
    /// slot at its declared position.
    // [spec:pgorm:sem:query.graph.cursor+4]
    pub(crate) fn qualifier(&self, index: usize) -> Option<Name> {
        self.qualifiers.get(index).cloned()
    }

    /// Join a hop that is never decoded — a junction table, a hop of a chain.
    ///
    /// `rel.to_tbl` is `LEFT JOIN`ed under the relation's own condition
    /// (`on_condition` and `condition_type` included), but the hop
    /// contributes nothing to the projection or the decode tuple and
    /// consumes no prefix index: joined because the path runs through it,
    /// invisible because nobody asked to read it.
    ///
    /// `LEFT` so that a missing middle cannot erase root rows by itself; a
    /// [`Req`] slot joined through it re-tightens the chain, because the
    /// `INNER` join's `ON` references the middle's columns and NULLs do not
    /// satisfy it.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, cake_filling, filling}};
    ///
    /// assert_eq!(
    ///     cake::Entity::graph()
    ///         .via(cake_filling::Relation::Cake.def().rev())
    ///         .join_maybe::<filling::Entity>(cake_filling::Relation::Filling.def())
    ///         .as_query()
    ///         .to_string()
    ///         .contains(r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#),
    ///     true
    /// );
    /// ```
    // [spec:pgorm:def:query.graph+2]
    #[doc(alias("find_also_linked", "find_with_linked"))]
    pub fn via(mut self, rel: RelationDef) -> Self {
        self.edge(JoinType::LeftJoin, rel, None, None);
        self
    }

    /// [`via`](Self::via) with the hop's joined side re-bound to a
    /// caller-supplied alias, which is how a path passes through one table
    /// twice.
    ///
    /// The alias is then the hop's one identifier: the `ON` condition's
    /// right side, and the name the next edge along the path starts from.
    /// Point that edge's relation at it with [`RelationDef::from_alias`];
    /// left alone, its `ON` names the table under its own name, which is
    /// the hop's first visit or no table in the query at all. The hop is
    /// otherwise `via`'s: joined `LEFT`, projected nowhere, consuming no
    /// prefix index. Distinctness is not checked client-side, on the terms
    /// [`join_maybe_as`](Self::join_maybe_as) states.
    ///
    /// The cakes that share a filling with each cake pass through the
    /// junction twice, and reach `cake` a second time:
    ///
    /// ```
    /// use pgorm::{alias, entity::*, query::*, tests_cfg::{cake, cake_filling, filling}};
    ///
    /// let shared = alias("shared");
    ///
    /// assert_eq!(
    ///     cake::Entity::graph()
    ///         .via(cake_filling::Relation::Cake.def().rev())
    ///         .via(cake_filling::Relation::Filling.def())
    ///         .via_as(cake_filling::Relation::Filling.def().rev(), shared)
    ///         .join_maybe_as::<cake::Entity>(
    ///             cake_filling::Relation::Cake.def().from_alias(shared),
    ///             alias("sibling"),
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
    ///         r#""sibling"."id" AS "s1_id", "sibling"."name" AS "s1_name""#,
    ///         r#"FROM "cake""#,
    ///         r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
    ///         r#"LEFT JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
    ///         r#"LEFT JOIN "cake_filling" AS "shared" ON "filling"."id" = "shared"."filling_id""#,
    ///         r#"LEFT JOIN "cake" AS "sibling" ON "shared"."cake_id" = "sibling"."id""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:query.graph.aliases+2]
    #[doc(alias("find_also_linked", "find_with_linked"))]
    pub fn via_as(mut self, rel: RelationDef, alias: impl IntoName) -> Self {
        self.edge(JoinType::LeftJoin, rel, Some(alias.into_name()), None);
        self
    }

    /// Cut the read to its first `limit` rows — rows of the join product,
    /// not roots — or, given `None`, remove a limit set before.
    ///
    /// The `LIMIT` lands on the graph's one statement, so the row reads,
    /// [`all`](Self::all), [`one_opt`](Self::one_opt) and
    /// [`stream`](Self::stream), answer exactly the rows the SQL returns: a
    /// root whose many slot matched three rows spends three of the limit.
    /// `one_opt` narrows the limit to one row and never widens it, so
    /// `limit(0)` reads none.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit}};
    ///
    /// let sql = cake::Entity::graph()
    ///     .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    ///     .order_by_asc(cake::Column::Id)
    ///     .limit(10)
    ///     .offset(20)
    ///     .as_query()
    ///     .to_string();
    ///
    /// assert!(sql.ends_with(r#"ORDER BY "cake"."id" ASC LIMIT 10 OFFSET 20"#), "{sql}");
    /// ```
    ///
    /// The graph comes back [`Sliced`], which has none of the three reads
    /// that decide the row range themselves. The grouped read is not there,
    /// because a window of rows can end part-way through a root's children
    /// and the entry would look complete:
    ///
    /// ```compile_fail,E0599
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit}, DatabaseConnection};
    ///
    /// async fn first_ten(db: &DatabaseConnection) {
    ///     let _ = cake::Entity::graph()
    ///         .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    ///         .limit(10)
    ///         .all_grouped(db)
    ///         .await;
    /// }
    /// ```
    ///
    /// Neither is the paginator, whose pages would replace the window and
    /// whose count would ignore it:
    ///
    /// ```compile_fail,E0599
    /// use std::num::NonZeroU64;
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit}, DatabaseConnection};
    ///
    /// fn pages(db: &DatabaseConnection) {
    ///     let _ = cake::Entity::graph()
    ///         .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    ///         .limit(10)
    ///         .paginate(db, NonZeroU64::MIN);
    /// }
    /// ```
    ///
    /// Nor the cursor, whose `first` and `last` would replace the window, and
    /// past whose boundary an offset would skip rows on every page:
    ///
    /// ```compile_fail,E0599
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit}};
    ///
    /// let _ = cake::Entity::graph()
    ///     .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    ///     .offset(10)
    ///     .cursor_by(cake::Column::Id);
    /// ```
    ///
    /// The first `n` roots with all of their related rows is a root read
    /// and a load: `E::find().limit(n)`, then
    /// [`load_many`](crate::LoaderTrait::load_many) or `load_many_via`.
    // [spec:pgorm:sem:query.graph.slice]
    pub fn limit(mut self, limit: impl Into<Option<u64>>) -> SelectGraph<E, S, Sliced> {
        match limit.into() {
            Some(limit) => self.query.limit(limit),
            None => self.query.reset_limit(),
        };
        self.retype()
    }

    /// Skip the read's first `offset` rows — rows of the join product, not
    /// roots — or, given `None`, remove an offset set before.
    ///
    /// The terms are [`limit`](Self::limit)'s: the graph comes back
    /// [`Sliced`], and the row reads skip exactly the rows the SQL skips, so
    /// a read can start part-way through one root's rows.
    // [spec:pgorm:sem:query.graph.slice]
    pub fn offset(mut self, offset: impl Into<Option<u64>>) -> SelectGraph<E, S, Sliced> {
        match offset.into() {
            Some(offset) => self.query.offset(offset),
            None => self.query.reset_offset(),
        };
        self.retype()
    }

    /// THE one edge walker: bind the joined side to an alias if asked, join
    /// it with the relation's own condition — plus any call-site `ON`
    /// predicate, ANDed in rather than replacing — and answer the joined
    /// side's effective identifier.
    ///
    /// A hop and a slot are both this join; a slot is one whose joined side
    /// is then projected ([`slot_edge`](Self::slot_edge)).
    // [spec:pgorm:def:query.graph+2]
    // [spec:pgorm:sem:query.graph.slots+2]
    // [spec:pgorm:req:query.graph.aliases+2]
    fn edge(
        &mut self,
        join: JoinType,
        mut rel: RelationDef,
        alias: Option<Name>,
        extra: Option<OnCondition>,
    ) -> Name {
        if let Some(from) = unaliased(&rel.from_tbl)
            && let Some(visit) = self.visits.iter().find(|visit| {
                visit.apart && visit.table.as_ref().map(identity) == Some(identity(from))
            })
        {
            rel.from_tbl = rel.from_tbl.alias(Name::clone(&visit.qualifier));
        }
        let mut apart = false;
        if let Some(alias) = alias {
            rel.to_tbl = rel.to_tbl.alias(alias);
        } else if let Some(to) = unaliased(&rel.to_tbl) {
            let bare = to.table().to_string();
            let repeated = self.visits.iter().any(|visit| {
                visit.qualifier.to_string() == bare
                    && visit.table.as_ref().map(identity) != Some(identity(to))
            });
            if repeated {
                let taken =
                    |name: &str| self.visits.iter().any(|v| v.qualifier.to_string() == name);
                let alias = alias_apart(to, &taken);
                rel.to_tbl = rel.to_tbl.alias(alias);
                apart = true;
            }
        }
        let left = Name::clone(rel.from_tbl.qualifier());
        let qualifier = Name::clone(rel.to_tbl.qualifier());
        let to_tbl = rel.to_tbl.clone();
        self.visits.push(Visit {
            qualifier: Name::clone(&qualifier),
            table: to_tbl.table_name().cloned(),
            apart,
        });
        let mut condition = join_condition(rel);
        if let Some(extra) = extra {
            condition = Condition::all()
                .add(condition)
                .add(extra(left, Name::clone(&qualifier)));
        }
        self.query.join(join, to_tbl, condition);
        qualifier
    }

    /// Join a decoded source through [`edge`](Self::edge), then project it
    /// under the next prefix by the identifier the join bound it to.
    // [spec:pgorm:req:query.graph.aliases+2]
    fn slot_edge<F: EntityTrait>(
        &mut self,
        join: JoinType,
        rel: RelationDef,
        alias: Option<Name>,
        extra: Option<OnCondition>,
    ) {
        let qualifier = self.edge(join, rel, alias, extra);
        self.project::<F>(qualifier);
    }

    /// Re-type the graph after an edge or a slice has been written into the
    /// statement.
    fn retype<S2, R2>(self) -> SelectGraph<E, S2, R2> {
        SelectGraph {
            query: self.query,
            qualifiers: self.qualifiers,
            visits: self.visits,
            marker: PhantomData,
        }
    }
}

/// The table `item` names under its own name, with no alias bound.
fn unaliased(item: &FromItem) -> Option<&TableName> {
    match item {
        FromItem::Table(NamedTable { name, alias: None }) => Some(name),
        _ => None,
    }
}

/// Generate the edge methods for one slot-tuple arity.
///
/// Each invocation takes the *existing* slot list and produces the methods
/// that append one more slot to it. The bound on decoded sources is the set
/// of arities generated here — a seventh has no receiver impl, and raising
/// the ceiling is one more entry in the invocation below.
macro_rules! grow {
    ( $( $( #[doc = $example:literal] )* ( $( $prev:ident ),* ) ),+ $(,)? ) => {$(
        // [spec:pgorm:sem:query.graph.slots+2]
        impl<E: EntityTrait, R $(, $prev: Slot )*> SelectGraph<E, ( $( $prev, )* ), R> {
            /// `LEFT JOIN` a source that may be absent, decoded as
            /// `Option<F::Model>` through the absence witness.
            ///
            /// `rel.to_tbl` is joined under the relation's own condition,
            /// `on_condition` and `condition_type` included.
            // [spec:pgorm:sem:query.graph.slots+2]
            #[doc(alias("select_also", "find_also_linked"))]
            pub fn join_maybe<F: EntityTrait>(
                mut self,
                rel: RelationDef,
            ) -> SelectGraph<E, ( $( $prev, )* Opt<F>, ), R> {
                self.slot_edge::<F>(<Opt<F> as Slot>::JOIN, rel, None, None);
                self.retype()
            }

            /// [`join_maybe`](Self::join_maybe) with the joined side re-bound
            /// to a caller-supplied alias, which is how the same table enters
            /// a graph twice.
            ///
            /// The alias is then the slot's one identifier everywhere: the
            /// `ON` condition's right side, the projection qualifier, a
            /// cursor tiebreak's qualifier. Distinctness is not checked
            /// client-side — joining a table the graph already names,
            /// unaliased, renders SQL PostgreSQL refuses.
            // [spec:pgorm:req:query.graph.aliases+2]
            pub fn join_maybe_as<F: EntityTrait>(
                mut self,
                rel: RelationDef,
                alias: impl IntoName,
            ) -> SelectGraph<E, ( $( $prev, )* Opt<F>, ), R> {
                self.slot_edge::<F>(<Opt<F> as Slot>::JOIN, rel, Some(alias.into_name()), None);
                self.retype()
            }

            /// [`join_maybe`](Self::join_maybe) with an extra `ON` predicate
            /// ANDed in *in addition to* whatever `on_condition` the relation
            /// already carries.
            ///
            /// Where [`RelationDef::on_condition`] replaces, this composes, so
            /// a call-site narrowing cannot silently drop an authored
            /// predicate. `ON` versus `WHERE` is the point of its existence:
            /// under a `LEFT JOIN` a predicate in `ON` narrows which rows
            /// *match* — unmatched roots survive, decoding `None` — while the
            /// same predicate through [`filter`](crate::QueryFilter::filter)
            /// lands in `WHERE`, where an unmatched row's NULLs fail it and
            /// the join silently tightens to `INNER`.
            ///
            /// The closure receives the join's left and right identifiers;
            /// qualify from them rather than hardcoding a table name, or an
            /// aliased slot's predicate will name a table that is not in the
            /// query. There is no `join_one_filtered`: under an `INNER JOIN`
            /// the two placements select the same rows, and `filter` already
            /// spells it.
            // [spec:pgorm:req:query.graph.aliases+2]
            pub fn join_maybe_filtered<F, C>(
                mut self,
                rel: RelationDef,
                f: C,
            ) -> SelectGraph<E, ( $( $prev, )* Opt<F>, ), R>
            where
                F: EntityTrait,
                C: Fn(Name, Name) -> Condition + Send + Sync + 'static,
            {
                self.slot_edge::<F>(<Opt<F> as Slot>::JOIN, rel, None, Some(Box::new(f)));
                self.retype()
            }

            /// `INNER JOIN` a source that must match, decoded as a bare
            /// `F::Model` — no `Option` to unwrap for a join that cannot miss.
            // [spec:pgorm:sem:query.graph.slots+2]
            pub fn join_one<F: EntityTrait>(
                mut self,
                rel: RelationDef,
            ) -> SelectGraph<E, ( $( $prev, )* Req<F>, ), R> {
                self.slot_edge::<F>(<Req<F> as Slot>::JOIN, rel, None, None);
                self.retype()
            }

            /// [`join_one`](Self::join_one) with the joined side re-bound to a
            /// caller-supplied alias, on the terms
            /// [`join_maybe_as`](Self::join_maybe_as) states.
            // [spec:pgorm:req:query.graph.aliases+2]
            pub fn join_one_as<F: EntityTrait>(
                mut self,
                rel: RelationDef,
                alias: impl IntoName,
            ) -> SelectGraph<E, ( $( $prev, )* Req<F>, ), R> {
                self.slot_edge::<F>(<Req<F> as Slot>::JOIN, rel, Some(alias.into_name()), None);
                self.retype()
            }

            /// Fold the path [`Related<F>`] already describes into the graph
            /// as an optional source.
            ///
            /// When [`Related::via`] is `Some`, that junction relation is
            /// [`via`](Self::via)ed first and then [`Related::to`] is
            /// [`join_maybe`](Self::join_maybe)d — the whole related path in
            /// one call, junction hop included.
            // [spec:pgorm:sem:query.graph.slots+2]
            #[doc(alias("find_also_related", "find_with_related"))]
            pub fn related_maybe<F: EntityTrait>(
                self,
            ) -> SelectGraph<E, ( $( $prev, )* Opt<F>, ), R>
            where
                E: Related<F>,
            {
                let graph = match <E as Related<F>>::via() {
                    Some(via) => self.via(via),
                    None => self,
                };
                graph.join_maybe::<F>(<E as Related<F>>::to())
            }

            /// Fold the path [`Related<F>`] already describes into the graph
            /// as a required source: [`related_maybe`](Self::related_maybe)'s
            /// path, every hop of it `INNER`.
            ///
            /// When [`Related::via`] is `Some`, that junction relation is
            /// joined `INNER` — not [`via`](Self::via)'s `LEFT` — and then
            /// [`Related::to`] is [`join_one`](Self::join_one)d, so a root
            /// row with no related `F` is not in the result, and each row
            /// that is decodes `F` as a bare `F::Model`.
            $( #[doc = $example] )*
            // [spec:pgorm:sem:query.graph.slots+2]
            pub fn related_one<F: EntityTrait>(
                mut self,
            ) -> SelectGraph<E, ( $( $prev, )* Req<F>, ), R>
            where
                E: Related<F>,
            {
                if let Some(via) = <E as Related<F>>::via() {
                    self.edge(<Req<F> as Slot>::JOIN, via, None, None);
                }
                self.join_one::<F>(<E as Related<F>>::to())
            }
        }
    )+};
}

grow!(
    // The example rides the slotless receiver alone: a doctest written into
    // every generated impl would be compiled and run once per arity.
    #[doc = r##"
```
use pgorm::{entity::*, query::*, tests_cfg::{cake, filling}};
use pgorm::query::{Req, SelectGraph};

let graph: SelectGraph<cake::Entity, (Req<filling::Entity>,)> =
    cake::Entity::graph().related_one::<filling::Entity>();

assert_eq!(
    graph.as_query().to_string(),
    [
        r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
        r#""filling"."id" AS "s1_id", "filling"."name" AS "s1_name", "filling"."vendor_id" AS "s1_vendor_id""#,
        r#"FROM "cake""#,
        r#"INNER JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
        r#"INNER JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
    ]
    .join(" ")
);
```
"##]
    (),
    (S1),
    (S1, S2),
    (S1, S2, S3),
    (S1, S2, S3, S4),
    (S1, S2, S3, S4, S5),
);

// [spec:pgorm:def:query.graph+2]
impl<E: EntityTrait, S, R> QueryFilter for SelectGraph<E, S, R> {
    type QueryStatement = SelectStatement;

    fn query(&mut self) -> &mut SelectStatement {
        &mut self.query
    }
}

// [spec:pgorm:def:query.graph+2]
impl<E: EntityTrait, S, R> QueryOrder for SelectGraph<E, S, R> {
    type QueryStatement = SelectStatement;

    fn query(&mut self) -> &mut SelectStatement {
        &mut self.query
    }
}

// [spec:pgorm:def:query.graph+2]
impl<E: EntityTrait, S, R> QueryTrait for SelectGraph<E, S, R> {
    type QueryStatement = SelectStatement;

    fn query(&mut self) -> &mut SelectStatement {
        &mut self.query
    }

    fn as_query(&self) -> &SelectStatement {
        &self.query
    }

    fn into_query(self) -> SelectStatement {
        self.query
    }
}

// [spec:pgorm:def:query.graph+2/test]    construction projects the root under
// `s0_` and selects from its table, `via` joins without consuming a prefix,
// and the filter / order / query traits reach the same statement
// [spec:pgorm:sem:query.graph.slots+2/test]    the slot kind fixes the join
// type, `related_one` joins its whole path INNER, and the declared tuple
// grows to the generated ceiling
// [spec:pgorm:sem:query.graph.writer+4/test]    one prefixed block per decoded
// source, in declaration order, under the source's effective identifier
// [spec:pgorm:req:query.graph.aliases+2/test]    an `_as` slot is named by its
// alias everywhere, and `join_maybe_filtered` composes with the relation's
// own `on_condition` instead of replacing it
// [spec:pgorm:req:query.graph.aliases+2/test]    a `via_as` hop is named by its
// alias in its own `ON`, in an authored condition's right side and in the
// next edge's `ON` once that relation starts `from_alias`, and takes no prefix
// [spec:pgorm:sem:query.graph.slice/test]    `limit` and `offset` bind on the
// graph's one statement and re-type it `Sliced`, `None` removes either, and a
// sliced graph keeps every builder method
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_cfg::{cake, cake_filling, filling, fruit, lunch_set, vendor};
    use crate::{ColumnTrait, QueryFilter, RelationTrait};
    use pgorm_query::{Expr, IntoCondition, alias};
    use pretty_assertions::assert_eq;

    #[track_caller]
    fn assert_renders(actual: String, parts: &[&str]) {
        assert_eq!(actual, parts.join(" "));
    }

    #[test]
    fn root_is_projected_at_construction() {
        assert_eq!(
            cake::Entity::graph().as_query().to_string(),
            r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name" FROM "cake""#
        );
        assert_eq!(
            SelectGraph::<cake::Entity>::default()
                .as_query()
                .to_string(),
            SelectGraph::<cake::Entity>::new().as_query().to_string()
        );
    }

    #[test]
    fn enum_columns_keep_cast_and_plain_alias() {
        assert_renders(
            lunch_set::Entity::graph().as_query().to_string(),
            &[
                r#"SELECT "lunch_set"."id" AS "s0_id", "lunch_set"."name" AS "s0_name","#,
                r#"CAST("lunch_set"."tea" AS text) AS "s0_tea" FROM "lunch_set""#,
            ],
        );
    }

    #[test]
    fn three_sources_one_writer() {
        assert_renders(
            cake::Entity::graph()
                .via(cake_filling::Relation::Cake.def().rev())
                .join_maybe::<filling::Entity>(cake_filling::Relation::Filling.def())
                .join_maybe::<vendor::Entity>(filling::Relation::Vendor.def())
                .as_query()
                .to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
                r#""filling"."id" AS "s1_id", "filling"."name" AS "s1_name", "filling"."vendor_id" AS "s1_vendor_id","#,
                r#""vendor"."id" AS "s2_id", "vendor"."name" AS "s2_name""#,
                r#"FROM "cake""#,
                r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
                r#"LEFT JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
                r#"LEFT JOIN "vendor" ON "filling"."vendor_id" = "vendor"."id""#,
            ],
        );
    }

    #[test]
    fn required_slot_is_inner() {
        assert_renders(
            cake::Entity::graph()
                .join_one::<fruit::Entity>(cake::Relation::Fruit.def())
                .as_query()
                .to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
                r#""fruit"."id" AS "s1_id", "fruit"."name" AS "s1_name", "fruit"."cake_id" AS "s1_cake_id""#,
                r#"FROM "cake""#,
                r#"INNER JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
            ],
        );
    }

    #[test]
    fn related_maybe_walks_the_junction() {
        assert_renders(
            cake::Entity::graph()
                .related_maybe::<filling::Entity>()
                .as_query()
                .to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
                r#""filling"."id" AS "s1_id", "filling"."name" AS "s1_name", "filling"."vendor_id" AS "s1_vendor_id""#,
                r#"FROM "cake""#,
                r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
                r#"LEFT JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
            ],
        );
    }

    #[test]
    fn related_maybe_without_a_junction_is_one_join() {
        assert_eq!(
            cake::Entity::graph()
                .related_maybe::<fruit::Entity>()
                .as_query()
                .to_string(),
            cake::Entity::graph()
                .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
                .as_query()
                .to_string()
        );
    }

    #[test]
    fn related_one_joins_every_hop_inner() {
        assert_renders(
            cake::Entity::graph()
                .related_one::<filling::Entity>()
                .as_query()
                .to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
                r#""filling"."id" AS "s1_id", "filling"."name" AS "s1_name", "filling"."vendor_id" AS "s1_vendor_id""#,
                r#"FROM "cake""#,
                r#"INNER JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
                r#"INNER JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
            ],
        );
    }

    #[test]
    fn related_one_without_a_junction_is_join_one() {
        assert_eq!(
            cake::Entity::graph()
                .related_one::<fruit::Entity>()
                .as_query()
                .to_string(),
            cake::Entity::graph()
                .join_one::<fruit::Entity>(cake::Relation::Fruit.def())
                .as_query()
                .to_string()
        );
    }

    #[test]
    fn related_one_appends_after_existing_slots() {
        let graph = cake::Entity::graph()
            .related_maybe::<fruit::Entity>()
            .related_one::<filling::Entity>();

        assert_eq!(graph.qualifiers.len(), 3);
        let sql = graph.as_query().to_string();
        assert!(
            sql.contains(r#""filling"."id" AS "s2_id""#),
            "the required slot takes the next prefix: {sql}"
        );
    }

    #[test]
    fn same_entity_twice_with_call_site_on() {
        // The canned `TropicalFruit` relation cannot be used here: its
        // authored `on_condition` hard-qualifies `"fruit"` instead of using
        // its `right` parameter, so under an alias it names the wrong table.
        assert_renders(
            cake::Entity::graph()
                .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
                .join_maybe_as::<fruit::Entity>(
                    cake::Relation::Fruit.def().on_condition(|_left, right| {
                        Expr::col((right, fruit::Column::Name))
                            .like("%tropical%")
                            .into_condition()
                    }),
                    alias("tropical"),
                )
                .as_query()
                .to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
                r#""fruit"."id" AS "s1_id", "fruit"."name" AS "s1_name", "fruit"."cake_id" AS "s1_cake_id","#,
                r#""tropical"."id" AS "s2_id", "tropical"."name" AS "s2_name", "tropical"."cake_id" AS "s2_cake_id""#,
                r#"FROM "cake""#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
                r#"LEFT JOIN "fruit" AS "tropical" ON "cake"."id" = "tropical"."cake_id" AND "tropical"."name" LIKE '%tropical%'"#,
            ],
        );
    }

    #[test]
    fn required_slot_takes_an_alias_too() {
        assert_renders(
            cake::Entity::graph()
                .join_one_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("topping"))
                .as_query()
                .to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
                r#""topping"."id" AS "s1_id", "topping"."name" AS "s1_name", "topping"."cake_id" AS "s1_cake_id""#,
                r#"FROM "cake""#,
                r#"INNER JOIN "fruit" AS "topping" ON "cake"."id" = "topping"."cake_id""#,
            ],
        );
    }

    #[test]
    fn filtered_join_composes_with_the_authored_condition() {
        // `TropicalFruit` already carries an authored `on_condition`; the
        // call-site predicate is ANDed in beside it, not over it.
        let sql = cake::Entity::graph()
            .join_maybe_filtered::<fruit::Entity, _>(
                cake::Relation::TropicalFruit.def(),
                |_left, right| {
                    Expr::col((right, fruit::Column::Id))
                        .gt(10)
                        .into_condition()
                },
            )
            .as_query()
            .to_string();

        assert!(
            sql.contains(r#""fruit"."name" LIKE '%tropical%'"#),
            "the authored predicate survives: {sql}"
        );
        assert!(
            sql.contains(r#""fruit"."id" > 10"#),
            "the call-site predicate is added: {sql}"
        );
        assert!(sql.contains("LEFT JOIN"), "the slot stays optional: {sql}");
    }

    #[test]
    fn filtered_join_lands_in_on_not_where() {
        let sql = cake::Entity::graph()
            .join_maybe_filtered::<fruit::Entity, _>(cake::Relation::Fruit.def(), |_left, right| {
                Expr::col((right, fruit::Column::Name))
                    .like("%berry%")
                    .into_condition()
            })
            .as_query()
            .to_string();

        assert!(!sql.contains("WHERE"), "no WHERE clause is written: {sql}");
        assert_eq!(
            sql,
            [
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
                r#""fruit"."id" AS "s1_id", "fruit"."name" AS "s1_name", "fruit"."cake_id" AS "s1_cake_id""#,
                r#"FROM "cake""#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id" AND "fruit"."name" LIKE '%berry%'"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn filter_and_order_reach_the_statement() {
        assert_renders(
            cake::Entity::graph()
                .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
                .filter(cake::Column::Id.gt(1))
                .order_by_asc(cake::Column::Id)
                .as_query()
                .to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name","#,
                r#""fruit"."id" AS "s1_id", "fruit"."name" AS "s1_name", "fruit"."cake_id" AS "s1_cake_id""#,
                r#"FROM "cake""#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
                r#"WHERE "cake"."id" > 1"#,
                r#"ORDER BY "cake"."id" ASC"#,
            ],
        );
    }

    #[test]
    fn six_slots_are_declarable() {
        let graph = cake::Entity::graph()
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("f2"))
            .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("f3"))
            .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("f4"))
            .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("f5"))
            .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("f6"));

        assert_eq!(graph.qualifiers.len(), 7);
        let sql = graph.as_query().to_string();
        assert!(sql.contains(r#""f6"."cake_id" AS "s6_cake_id""#), "{sql}");
    }

    #[test]
    fn via_as_names_the_hop_by_its_alias() {
        let shared = alias("shared");
        let graph = cake::Entity::graph()
            .via(cake_filling::Relation::Cake.def().rev())
            .via(cake_filling::Relation::Filling.def())
            .via_as(cake_filling::Relation::Filling.def().rev(), shared)
            .via_as(
                cake_filling::Relation::Cake.def().from_alias(shared),
                alias("sibling"),
            );

        assert_eq!(graph.qualifiers.len(), 1, "a hop takes no prefix index");
        assert_renders(
            graph.as_query().to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name""#,
                r#"FROM "cake""#,
                r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
                r#"LEFT JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
                r#"LEFT JOIN "cake_filling" AS "shared" ON "filling"."id" = "shared"."filling_id""#,
                r#"LEFT JOIN "cake" AS "sibling" ON "shared"."cake_id" = "sibling"."id""#,
            ],
        );
    }

    #[test]
    fn via_as_hands_the_alias_to_the_condition() {
        let sql = cake::Entity::graph()
            .via_as(
                cake_filling::Relation::Cake
                    .def()
                    .rev()
                    .on_condition(|_left, right| {
                        Expr::col((right, cake_filling::Column::FillingId))
                            .gt(5)
                            .into_condition()
                    }),
                alias("cf"),
            )
            .as_query()
            .to_string();

        assert!(
            sql.ends_with(
                r#"LEFT JOIN "cake_filling" AS "cf" ON "cake"."id" = "cf"."cake_id" AND "cf"."filling_id" > 5"#
            ),
            "the authored predicate is qualified by the alias: {sql}"
        );
    }

    #[test]
    fn slot_after_aliased_hop_takes_next_prefix() {
        let graph = cake::Entity::graph()
            .via_as(cake_filling::Relation::Cake.def().rev(), alias("cf"))
            .join_one::<filling::Entity>(
                cake_filling::Relation::Filling
                    .def()
                    .from_alias(alias("cf")),
            );

        assert_eq!(graph.slot_qualifiers(), [Name::new(filling::Entity)]);
        let sql = graph.as_query().to_string();
        assert!(sql.contains(r#""filling"."id" AS "s1_id""#), "{sql}");
        assert!(
            sql.ends_with(r#"INNER JOIN "filling" ON "cf"."filling_id" = "filling"."id""#),
            "{sql}"
        );
    }

    #[test]
    fn limit_and_offset_land_on_the_statement() {
        let sliced: SelectGraph<cake::Entity, (Opt<fruit::Entity>,), Sliced> =
            cake::Entity::graph()
                .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
                .order_by_asc(cake::Column::Id)
                .limit(2)
                .offset(1);

        assert!(
            sliced
                .as_query()
                .to_string()
                .ends_with(r#"ORDER BY "cake"."id" ASC LIMIT 2 OFFSET 1"#),
            "{}",
            sliced.as_query().to_string()
        );
        assert_eq!(
            sliced.as_query().build().1,
            pgorm_query::Values(vec![2u64.into(), 1u64.into()]),
            "the window binds, as every other limit does"
        );
    }

    #[test]
    fn none_removes_a_limit_or_an_offset() {
        let sql = cake::Entity::graph()
            .limit(3)
            .offset(4)
            .limit(None)
            .offset(None)
            .as_query()
            .to_string();

        assert_eq!(sql, cake::Entity::graph().as_query().to_string());
    }

    #[test]
    fn a_sliced_graph_keeps_growing() {
        let graph = cake::Entity::graph()
            .limit(5)
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .filter(fruit::Column::Id.gt(1))
            .limit(2);

        let sql = graph.as_query().to_string();
        assert!(sql.contains(r#""fruit"."id" AS "s1_id""#), "{sql}");
        assert!(
            sql.ends_with(r#"WHERE "fruit"."id" > 1 LIMIT 2"#),
            "the later limit replaces the earlier: {sql}"
        );
    }

    #[test]
    fn via_hops_consume_no_prefix() {
        let graph = cake::Entity::graph()
            .via(cake_filling::Relation::Cake.def().rev())
            .via(cake_filling::Relation::Filling.def());

        assert_eq!(graph.qualifiers.len(), 1);
        assert_renders(
            graph.as_query().to_string(),
            &[
                r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name""#,
                r#"FROM "cake""#,
                r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
                r#"LEFT JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
            ],
        );
    }

    // [spec:pgorm:req:query.graph.aliases+2/test]    a slot whose bare name the graph already qualifies another table by is joined under its schema-qualified name as one identifier, its projection and `ON` following, an edge leaving from it leaves from that name, and the same table joined twice stays the caller's to alias
    #[test]
    fn same_named_tables_are_aliased_apart() {
        use crate::tests_cfg::{archive_exhibit, gallery_exhibit, loan_exhibit};
        let loan = archive_exhibit::Entity::belongs_to(loan_exhibit::Entity)
            .columns(archive_exhibit::Column::Id, loan_exhibit::Column::Id)
            .into();
        assert_renders(
            gallery_exhibit::Entity::graph()
                .join_maybe::<archive_exhibit::Entity>(gallery_exhibit::Relation::Archived.def())
                .join_maybe::<loan_exhibit::Entity>(loan)
                .as_query()
                .to_string(),
            &[
                r#"SELECT "exhibit"."id" AS "s0_id", CAST("exhibit"."kind" AS text) AS "s0_kind", "exhibit"."title" AS "s0_title","#,
                r#""archive.exhibit"."id" AS "s1_id", "archive.exhibit"."shelf" AS "s1_shelf","#,
                r#""loan.exhibit"."id" AS "s2_id", "loan.exhibit"."lender" AS "s2_lender""#,
                r#"FROM "gallery"."exhibit""#,
                r#"LEFT JOIN "archive"."exhibit" AS "archive.exhibit" ON "exhibit"."id" = "archive.exhibit"."id""#,
                r#"LEFT JOIN "loan"."exhibit" AS "loan.exhibit" ON "archive.exhibit"."id" = "loan.exhibit"."id""#,
            ],
        );

        let twice = cake::Entity::graph()
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .as_query()
            .to_string();
        assert!(
            twice.ends_with(concat!(
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id" "#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#
            )),
            "{twice}"
        );
    }
}
