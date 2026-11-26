//! Decoding and terminating an N-ary relational read.
//!
//! [`GraphRow<E, S>`] is the [`DecodeRow`] a [`SelectGraph<E, S>`]
//! converts into, and the terminals are that conversion plus the ordinary
//! [`Selector`] machinery. The graph adds a declaration layer, not a second
//! execution path.

use core::marker::PhantomData;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;
use std::num::NonZeroU64;

use pgorm_query::{Order, Value};

use crate::{
    Connection, DecodeRow, Entity, Error, FromQueryResult, Iterable, Model, Opt, Paginate,
    Paginator, PinBoxSendStream, PrimaryKeyToColumn, QueryResult, SelectGraph, Selector, Slot,
    Unsliced,
};

/// The selector that decodes one graph row: the root under `s0_`, then each
/// slot in declaration order under its own `s{i}_` prefix.
///
/// The item is `(E::Model, S1::Out, …, Sn::Out)`, computed from the declared
/// slot tuple; a slotless graph decodes as a bare `E::Model`, not a
/// one-tuple. `Req` slots decode through
/// [`FromQueryResult::from_query_result`], `Opt` slots through
/// [`FromQueryResult::from_query_result_optional`] — the absence witness,
/// unchanged, of which the graph is the N-ary consumer.
///
/// The type is nameable because the cursor and paginator signatures need it;
/// it carries no data and is not constructed by callers.
// [spec:pgorm:sem:query.graph.decode+2]
pub struct GraphRow<E, S>(PhantomData<(E, S)>);

impl<E, S> fmt::Debug for GraphRow<E, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GraphRow")
    }
}

impl<E, S> Clone for GraphRow<E, S> {
    fn clone(&self) -> Self {
        GraphRow(PhantomData)
    }
}

impl<E, S> GraphRow<E, S> {
    /// The witness value a [`Selector`] carries; it holds nothing.
    pub(crate) fn new() -> Self {
        GraphRow(PhantomData)
    }
}

/// A slotless graph is a single-entity read: one model, no tuple.
// [spec:pgorm:sem:query.graph.decode+2]
impl<E: Entity> DecodeRow for GraphRow<E, ()> {
    type Item = E::Model;

    fn from_raw_query_result(res: QueryResult) -> Result<Self::Item, Error> {
        E::Model::from_query_result(&res, ROOT_PREFIX)
    }
}

/// The prefix the writer aliases the root's columns under.
const ROOT_PREFIX: &str = "s0_";

/// Generate the row decode for one slot-tuple arity.
///
/// The `?` on each source is the abort: the first failing source's error is
/// the row's error, and later sources are not examined.
macro_rules! graph_row {
    ( $( $s:ident @ $pre:literal ),+ ) => {
        // [spec:pgorm:sem:query.graph.decode+2]
        impl<E: Entity, $( $s: Slot ),+> DecodeRow for GraphRow<E, ( $( $s, )+ )> {
            type Item = (E::Model, $( $s::Out ),+ );

            fn from_raw_query_result(res: QueryResult) -> Result<Self::Item, Error> {
                Ok((
                    E::Model::from_query_result(&res, ROOT_PREFIX)?,
                    $( $s::decode(&res, $pre)? ),+
                ))
            }
        }
    };
}

graph_row!(S1 @ "s1_");
graph_row!(S1 @ "s1_", S2 @ "s2_");
graph_row!(S1 @ "s1_", S2 @ "s2_", S3 @ "s3_");
graph_row!(S1 @ "s1_", S2 @ "s2_", S3 @ "s3_", S4 @ "s4_");
graph_row!(S1 @ "s1_", S2 @ "s2_", S3 @ "s3_", S4 @ "s4_", S5 @ "s5_");
graph_row!(S1 @ "s1_", S2 @ "s2_", S3 @ "s3_", S4 @ "s4_", S5 @ "s5_", S6 @ "s6_");

/// What one row of a graph decodes into.
pub type GraphItem<E, S> = <GraphRow<E, S> as DecodeRow>::Item;

// [spec:pgorm:sem:query.graph.terminals+3]
// [spec:pgorm:sem:query.graph.slice+1]
impl<E: Entity, S, R, Cx> SelectGraph<E, S, R, Cx>
where
    GraphRow<E, S>: DecodeRow,
{
    /// Convert into the [`Selector`] every terminal runs through.
    ///
    /// Everything past this conversion is machinery specified elsewhere; the
    /// graph duplicates none of it.
    // [spec:pgorm:sem:query.graph.terminals+3]
    pub(crate) fn into_selector(self) -> Selector<GraphRow<E, S>> {
        Selector {
            query: self.query,
            selector: GraphRow::new(),
            refused: self.refused,
        }
    }

    /// Fetch every row as the declared tuple.
    ///
    /// A decode failure aborts the read at the first bad row, as on every
    /// other model-typed `all`.
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::{cake, fruit}, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// let rows: Vec<(cake::Model, Option<fruit::Model>)> = cake::Entity::graph()
    ///     .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    ///     .all(&db)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    // [spec:pgorm:sem:query.graph.terminals+3]
    pub async fn all<C: Connection>(self, db: &C) -> Result<Vec<GraphItem<E, S>>, Error> {
        self.into_selector().all(db).await
    }

    /// Fetch the first row, or `None` when the query returns none.
    ///
    /// The row limit is narrowed to one row and never widened, as on
    /// [`Selector::one_opt`], so a graph limited to no row reads none and an
    /// offset is kept. There is no `one`: a graph row is a join product, so
    /// "exactly one" is a claim about the product rather than about the root.
    // [spec:pgorm:sem:query.graph.terminals+3]
    pub async fn one_opt<C: Connection>(self, db: &C) -> Result<Option<GraphItem<E, S>>, Error> {
        self.into_selector().one_opt(db).await
    }

    /// Stream rows as declared tuples, decoding lazily per item.
    ///
    /// A row that fails to decode is yielded as one `Err` item and the stream
    /// continues, as on every other model-typed stream.
    // [spec:pgorm:sem:query.graph.terminals+3]
    pub async fn stream<'b, C: Connection>(
        self,
        db: &C,
    ) -> Result<PinBoxSendStream<'b, Result<GraphItem<E, S>, Error>>, Error>
    where
        GraphRow<E, S>: 'b,
    {
        self.into_selector().stream(db).await
    }
}

/// The grouping key: the root's decoded primary-key value, at whatever arity
/// the key has.
///
/// A unary key — the overwhelming case — is carried inline, so grouping a
/// result set allocates nothing per row for it.
// [spec:pgorm:sem:query.graph.grouped+2]
#[derive(PartialEq, Eq, Hash)]
enum RootKey {
    /// A single-column primary key.
    Unit(Value),
    /// A composite primary key, in `PrimaryKey` iteration order.
    Composite(Vec<Value>),
}

/// Read the root's primary-key value out of a *decoded* model — the grouping
/// is keyed on what the row said the root is, not on where the row sat.
// [spec:pgorm:sem:query.graph.grouped+2]
fn root_key<E: Entity>(model: &E::Model) -> RootKey {
    let mut keys = <E::PrimaryKey as Iterable>::iter();
    match (keys.next(), keys.next()) {
        (Some(only), None) => RootKey::Unit(model.get(only.into_column())),
        (first, second) => RootKey::Composite(
            first
                .into_iter()
                .chain(second)
                .chain(keys)
                .map(|pk| model.get(pk.into_column()))
                .collect(),
        ),
    }
}

/// Consolidate decoded rows into one entry per distinct root key.
///
/// One pass in row order: a key not seen before appends its root with an empty
/// child list, so entries sit at their first occurrence; a key seen before
/// finds that entry again, so a run torn apart by the ordering merges rather
/// than emitting the root twice. Children are pushed as they arrive, and a row
/// whose slot decoded `None` contributes nothing beyond its root.
// [spec:pgorm:sem:query.graph.grouped+2]
fn group_rows<E: Entity, F: Entity>(
    rows: Vec<(E::Model, Option<F::Model>)>,
) -> Vec<(E::Model, Vec<F::Model>)> {
    let mut grouped: Vec<(E::Model, Vec<F::Model>)> = Vec::new();
    let mut seen: HashMap<RootKey, usize> = HashMap::new();

    for (root, child) in rows {
        let at = match seen.entry(root_key::<E>(&root)) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let at = grouped.len();
                entry.insert(at);
                grouped.push((root, Vec::new()));
                at
            }
        };
        if let Some(child) = child {
            grouped[at].1.push(child);
        }
    }

    grouped
}

/// The grouped read, which exists on exactly one shape: a graph whose slot
/// tuple is `(Opt<F>,)`.
// [spec:pgorm:sem:query.graph.grouped+2]
impl<E: Entity, F: Entity, Cx> SelectGraph<E, (Opt<F>,), Unsliced, Cx> {
    /// Fetch every root once, with the slot's matching models collected
    /// beneath it.
    ///
    /// The method exists only here — one optional slot beside the root,
    /// `via()` hops permitted, which is the shape a junction-mediated has-many
    /// takes. Asking for it on any other slot tuple is a compile error, so a
    /// grouped read over two slots — whose meaning is not defined — cannot be
    /// written:
    ///
    /// ```compile_fail,E0599
    /// use pgorm::{alias, entity::*, query::*, tests_cfg::{cake, fruit}, DatabaseConnection};
    ///
    /// async fn two_slots(db: &DatabaseConnection) {
    ///     let _ = cake::Entity::graph()
    ///         .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    ///         .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("other"))
    ///         .all_grouped(db)
    ///         .await;
    /// }
    /// ```
    ///
    /// **Caller ordering dominates.** `E`'s primary-key columns, qualified
    /// with `E`'s table, are appended ascending as *trailing* `ORDER BY` keys
    /// — after every ordering the caller wrote, never before it. Order by
    /// nothing and the result is pure primary-key order; order by anything and
    /// that ordering stands, with the key appended as a deterministic tiebreak
    /// only.
    ///
    /// Grouping is keyed, not adjacency-based: rows consolidate on the root's
    /// decoded primary-key value. Each distinct key yields exactly one entry,
    /// positioned at its first occurrence in row order; children are pushed in
    /// row order, so the caller's ordering orders each bucket too; a root the
    /// slot did not match reads as an empty `Vec`. An ordering that
    /// interleaves roots therefore merges the torn run into the entry at its
    /// first appearance rather than repeating the root.
    ///
    /// There is no paginated form, for the reason page boundaries fall between
    /// rows rather than between roots, and no sliced one: the method is not on
    /// a graph [`limit`](SelectGraph::limit) or [`offset`](SelectGraph::offset)
    /// has cut, whose window of rows can end part-way through a root's
    /// children.
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::{cake, fruit}, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// let cakes: Vec<(cake::Model, Vec<fruit::Model>)> = cake::Entity::graph()
    ///     .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    ///     .order_by_desc(cake::Column::Name)
    ///     .all_grouped(&db)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    // [spec:pgorm:sem:query.graph.grouped+2]
    #[doc(alias(
        "find_with_related",
        "find_with_linked",
        "select_with",
        "SelectTwoMany"
    ))]
    pub async fn all_grouped<C: Connection>(
        self,
        db: &C,
    ) -> Result<Vec<(E::Model, Vec<F::Model>)>, Error> {
        let rows = self.key_ordered().into_selector().all(db).await?;
        Ok(group_rows::<E, F>(rows))
    }

    /// Append `E`'s primary-key columns, qualified with `E`'s table, ascending
    /// — behind whatever the caller already ordered by.
    // [spec:pgorm:sem:query.graph.grouped+2]
    fn key_ordered(mut self) -> Self {
        for pk in <E::PrimaryKey as Iterable>::iter() {
            self.query
                .order_by((E::default(), pk.into_column()), Order::Asc);
        }
        self
    }
}

/// Pagination reaches the graph through the same selector: page boundaries
/// fall between *rows*, not between root models, so a root with several
/// matching slot rows spans pages exactly as the underlying SQL does.
///
/// Only an unsliced graph paginates: a page's `LIMIT` and `OFFSET` would
/// replace a slice's, and the count strips them.
// [spec:pgorm:sem:query.graph.terminals+3]
// [spec:pgorm:sem:query.graph.slice+1]
impl<'db, C, E, S, Cx> Paginate<'db, C> for SelectGraph<E, S, Unsliced, Cx>
where
    C: Connection,
    E: Entity,
    GraphRow<E, S>: DecodeRow + Send + Sync + 'db,
{
    type Selector = GraphRow<E, S>;

    fn paginate(self, db: &'db C, page_size: NonZeroU64) -> Paginator<'db, C, Self::Selector> {
        self.into_selector().paginate(db, page_size)
    }
}

// [spec:pgorm:sem:query.graph.grouped+2/test]    the primary key is appended
// behind the caller's ordering rather than in front of it, at any key arity,
// and the consolidation keys on the decoded root instead of on adjacency
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_cfg::{cake, cake_filling, filling, fruit};
    use crate::{QueryOrder, Relation, Statement};
    use pretty_assertions::assert_eq;

    fn cake_of(id: i32, name: &str) -> cake::Model {
        cake::Model {
            id,
            name: name.to_owned(),
        }
    }

    fn fruit_of(id: i32, cake_id: i32) -> fruit::Model {
        fruit::Model {
            id,
            name: format!("fruit-{id}"),
            cake_id: Some(cake_id),
        }
    }

    #[track_caller]
    fn assert_order(sql: &str, tail: &str) {
        assert!(sql.ends_with(tail), "expected to end with `{tail}`: {sql}");
    }

    #[test]
    fn an_unordered_read_gets_pure_key_order() {
        assert_order(
            &cake::Entity::graph()
                .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
                .key_ordered()
                .as_query()
                .to_string(),
            r#"ORDER BY "cake"."id" ASC"#,
        );
    }

    #[test]
    fn caller_ordering_dominates_the_key() {
        assert_order(
            &cake::Entity::graph()
                .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
                .order_by_desc(cake::Column::Name)
                .order_by_asc(fruit::Column::Id)
                .key_ordered()
                .as_query()
                .to_string(),
            r#"ORDER BY "cake"."name" DESC, "fruit"."id" ASC, "cake"."id" ASC"#,
        );
    }

    #[test]
    fn every_key_column_appends_root_qualified() {
        assert_order(
            &cake_filling::Entity::graph()
                .join_maybe::<filling::Entity>(cake_filling::Relation::Filling.def())
                .order_by_desc(filling::Column::Name)
                .key_ordered()
                .as_query()
                .to_string(),
            r#"ORDER BY "filling"."name" DESC, "cake_filling"."cake_id" ASC, "cake_filling"."filling_id" ASC"#,
        );
    }

    #[test]
    fn grouping_keys_on_the_root_not_on_adjacency() {
        // A torn run: cake 1 appears either side of cake 3, and cake 2 matched
        // nothing at all.
        let grouped = group_rows::<cake::Entity, fruit::Entity>(vec![
            (cake_of(1, "Cheesecake"), Some(fruit_of(10, 1))),
            (cake_of(3, "Mudcake"), Some(fruit_of(11, 3))),
            (cake_of(1, "Cheesecake"), Some(fruit_of(12, 1))),
            (cake_of(2, "Lonely"), None),
        ]);

        assert_eq!(
            grouped,
            [
                // One entry, at the first occurrence, holding both children in
                // row order.
                (
                    cake_of(1, "Cheesecake"),
                    vec![fruit_of(10, 1), fruit_of(12, 1)]
                ),
                (cake_of(3, "Mudcake"), vec![fruit_of(11, 3)]),
                (cake_of(2, "Lonely"), vec![]),
            ]
        );
    }

    #[test]
    fn a_composite_key_groups_on_every_column() {
        let rows = vec![
            (
                cake_filling::Model {
                    cake_id: 1,
                    filling_id: 5,
                },
                None,
            ),
            (
                cake_filling::Model {
                    cake_id: 1,
                    filling_id: 6,
                },
                None,
            ),
            (
                cake_filling::Model {
                    cake_id: 1,
                    filling_id: 5,
                },
                None,
            ),
        ];

        // Two distinct pairs, not one group keyed on `cake_id` alone.
        assert_eq!(
            group_rows::<cake_filling::Entity, filling::Entity>(rows).len(),
            2
        );
    }
}
