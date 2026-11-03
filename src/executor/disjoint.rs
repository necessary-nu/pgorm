//! Decoding and terminating a disjoint entity's read.
//!
//! [`DisjointRow<D>`] is the [`SelectorTrait`] a [`SelectDisjoint<D>`]
//! converts into, and every terminal is that conversion plus the ordinary
//! [`Selector`], [`Paginator`] and [`Cursor`] machinery, after
//! [`check_disjoint`] has passed.

use core::marker::PhantomData;
use std::fmt;
use std::num::NonZeroU64;

use pgorm_query::Name;

use crate::entity::absent_row;
use crate::error::query_err;
use crate::query::BASE_SOURCE;
use crate::query::graph::source_prefix;
use crate::{
    ActiveEnum, ConnectionTrait, Cursor, CursorTrait, Disjoint, EntityTrait, Error,
    FromQueryResult, Iterable, KeyOf, KindModel, KindVisitor, ModelTrait, Paginator,
    PaginatorTrait, PinBoxSendStream, PrimaryKeyToColumn, QueryResult, SelectDisjoint, Selector,
    SelectorTrait, StaticName, check_disjoint, table_label,
};

/// The selector that decodes one row of a disjoint read: the base's model
/// under `s0_`, and the value of `D` its kind gives.
///
/// The row's kind is its base row's. A kind without a table is its unit
/// variant; a kind with one decodes the row joined from its table, through
/// the absence witness, into its variant. A row of a kind with a table whose
/// joined row is absent, and a row with a joined row present for any other
/// kind, are each an error naming the base table, the key and the kinds
/// involved: neither is a state the constraints `Schema::create_disjoint`
/// writes admit, so neither is read as a variant without its row, nor
/// dropped.
///
/// The type is nameable because the cursor and paginator signatures need it;
/// it carries no data and is not constructed by callers.
// [spec:pgorm:sem:disjoint.read+1]
pub struct DisjointRow<D>(PhantomData<fn() -> D>);

impl<D> fmt::Debug for DisjointRow<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DisjointRow")
    }
}

impl<D> Clone for DisjointRow<D> {
    fn clone(&self) -> Self {
        DisjointRow(PhantomData)
    }
}

/// What one row of a disjoint read decodes into.
pub type DisjointItem<D> = (<<D as Disjoint>::Base as EntityTrait>::Model, D);

// [spec:pgorm:sem:disjoint.read+1]
impl<D: Disjoint> SelectorTrait for DisjointRow<D> {
    type Item = DisjointItem<D>;

    fn from_raw_query_result(res: QueryResult) -> Result<Self::Item, Error> {
        let base_prefix = source_prefix(BASE_SOURCE);
        let base = <D::Base as EntityTrait>::Model::from_query_result(&res, &base_prefix)?;
        let label: String = res.try_get(&base_prefix, D::kind_column().as_str())?;
        let kind = D::Kind::try_from_value(&label)?;

        let mut sources = BASE_SOURCE;
        let mut value = None;
        let mut missing = None;
        let mut strays = Vec::new();
        for visited in D::Kind::iter() {
            let own = visited == kind;
            let read = ReadKind {
                res: &res,
                sources: &mut sources,
                decode: own,
            };
            match D::visit_kind(visited, read)? {
                Joined::Unit(unit) if own => value = Some(unit),
                Joined::Absent { table } if own => missing = Some(table),
                Joined::Unit(_) | Joined::Absent { .. } => {}
                Joined::Present {
                    decoded: Some(decoded),
                    ..
                } => value = Some(decoded),
                Joined::Present {
                    decoded: None,
                    table,
                } => strays.push((visited, table)),
            }
        }

        match value {
            Some(value) if strays.is_empty() => Ok((base, value)),
            _ => Err(inconsistent_row::<D>(&base, kind, missing, &strays)),
        }
    }
}

/// What one kind's part of a row holds.
enum Joined<D> {
    /// A kind with no table, as its value.
    Unit(D),
    /// A kind with a table, its joined row absent.
    Absent { table: String },
    /// A kind with a table, its joined row present: decoded into its variant
    /// when it is the row's kind, and only witnessed otherwise.
    Present { decoded: Option<D>, table: String },
}

/// Reads one kind's part of a row under the prefix its table was projected
/// with.
///
/// `sources` counts the kinds with a table visited so far, as the projection
/// counted them (`SelectDisjoint`'s visit over the same kinds in the same
/// order), so each kind reads under the prefix it was projected under. Only
/// the row's own kind is decoded; any other's row is judged by the absence
/// witness alone, so a stray row is reported as stray even if it would not
/// decode.
struct ReadKind<'r> {
    res: &'r QueryResult,
    sources: &'r mut usize,
    decode: bool,
}

impl<D: Disjoint> KindVisitor<D> for ReadKind<'_> {
    type Output = Result<Joined<D>, Error>;

    // [spec:pgorm:sem:disjoint.read+1]
    fn table<M: KindModel<D>>(self, _: D::Kind) -> Self::Output {
        *self.sources += 1;
        let prefix = source_prefix(*self.sources);
        let table = table_label(&M::Entity::default());
        if self.decode {
            return Ok(match M::from_query_result_optional(self.res, &prefix)? {
                Some(model) => Joined::Present {
                    decoded: Some(model.into_variant()),
                    table,
                },
                None => Joined::Absent { table },
            });
        }
        Ok(if absent_row::<M>(self.res, &prefix) {
            Joined::Absent { table }
        } else {
            Joined::Present {
                decoded: None,
                table,
            }
        })
    }

    fn unit(self, value: D) -> Self::Output {
        Ok(Joined::Unit(value))
    }
}

/// The error for a row no schema `Schema::create_disjoint` wrote can hold: a
/// row of a kind with a table but no row in it, or with a row in another
/// kind's table.
///
/// It is [`Error::Query`], as `load_one`'s report of a key a `HasOne`
/// relation matched twice is: the declaration is sound — `check_disjoint`
/// passed, and refuses as [`Error::Type`] what it does not — and the rows the
/// server returned contradict it, which only a schema other than the one the
/// declaration describes can do.
// [spec:pgorm:sem:disjoint.read+1]
fn inconsistent_row<D: Disjoint>(
    base: &<D::Base as EntityTrait>::Model,
    kind: D::Kind,
    missing: Option<String>,
    strays: &[(D::Kind, String)],
) -> Error {
    let key: Vec<String> = <D::Base as EntityTrait>::PrimaryKey::iter()
        .map(|key| {
            let column = key.into_column();
            format!("`{}` = {}", column.as_str(), base.get(column))
        })
        .collect();
    let mut faults = Vec::new();
    if let Some(table) = missing {
        faults.push(format!("no row in `{table}`, its kind's table"));
    }
    for (stray, table) in strays {
        faults.push(format!(
            "a row in `{table}`, the table of the kind `{}`",
            stray.to_value()
        ));
    }
    query_err(format!(
        "the `{base_table}` row whose key is ({key}) is of the kind `{kind}` but has {faults}: \
         the constraints `Schema::create_disjoint` writes hold each base row to exactly its own \
         kind's table, so this schema was not created by it",
        base_table = table_label(&D::Base::default()),
        key = key.join(", "),
        kind = kind.to_value(),
        faults = faults.join(" and "),
    ))
}

// [spec:pgorm:sem:disjoint.read+1]
impl<D: Disjoint> SelectDisjoint<D> {
    /// Convert into the [`Selector`] every terminal runs through, once the
    /// declaration has passed [`check_disjoint`].
    // [spec:pgorm:sem:disjoint.read+1]
    fn into_selector(self) -> Result<Selector<DisjointRow<D>>, Error> {
        check_disjoint::<D>()?;
        Ok(Selector {
            query: self.query,
            selector: DisjointRow(PhantomData),
        })
    }

    /// Fetch the first row, failing with [`Error::RecordNotFound`] when the
    /// read matches none.
    ///
    /// The row limit is narrowed to one row, as on [`Selector::one`]:
    /// `find_by_id(key).one(db)` is the row of that key.
    // [spec:pgorm:sem:disjoint.read+1]
    pub async fn one<C: ConnectionTrait>(self, db: &C) -> Result<DisjointItem<D>, Error> {
        self.into_selector()?.one(db).await
    }

    /// Fetch every row, as on [`Selector::all`]: the first row that fails to
    /// decode, an inconsistent one included, is the read's error.
    // [spec:pgorm:sem:disjoint.read+1]
    pub async fn all<C: ConnectionTrait>(self, db: &C) -> Result<Vec<DisjointItem<D>>, Error> {
        self.into_selector()?.all(db).await
    }

    /// Stream the rows, decoding each as it arrives, as on
    /// [`Selector::stream`]: a row that fails to decode, an inconsistent one
    /// included, is one `Err` item and the stream goes on.
    // [spec:pgorm:sem:disjoint.read+1]
    pub async fn stream<'b, C: ConnectionTrait>(
        self,
        db: &C,
    ) -> Result<PinBoxSendStream<'b, Result<DisjointItem<D>, Error>>, Error>
    where
        D: 'b,
    {
        self.into_selector()?.stream(db).await
    }

    /// Page through the read, as [`PaginatorTrait::paginate`] pages a
    /// [`Select`](crate::Select): each page the statement with its own
    /// `LIMIT` and `OFFSET`, replacing any the read set.
    ///
    /// Fallible where `PaginatorTrait::paginate` is not, because
    /// [`check_disjoint`] runs before anything is sent, and here that is
    /// before the paginator exists.
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn paginate<'db, C>(
        self,
        db: &'db C,
        page_size: NonZeroU64,
    ) -> Result<Paginator<'db, C, DisjointRow<D>>, Error>
    where
        C: ConnectionTrait,
        D: 'db,
    {
        Ok(self.into_selector()?.paginate(db, page_size))
    }

    /// Count the base rows the read matches, as [`PaginatorTrait::count`]
    /// does.
    // [spec:pgorm:sem:disjoint.read+1]
    pub async fn count<C: ConnectionTrait>(self, db: &C) -> Result<u64, Error> {
        self.into_selector()?.count(db).await
    }

    /// Convert into a keyset cursor ordered by columns of the base, as
    /// [`Select::cursor_by`](crate::Select::cursor_by) converts a select of
    /// it: the order columns qualified with the base's table, their boundary
    /// values bound under the columns' casts.
    ///
    /// Each base row is one row of the read, so the base's order columns are
    /// the whole keyset; an order on a column no two base rows share needs
    /// no tiebreak. Fallible for the reason [`paginate`](Self::paginate) is.
    ///
    /// ```
    /// # use pgorm::{entity::prelude::*, tests_cfg::vehicle::{self, Vehicle}};
    /// # fn example() -> Result<(), Error> {
    /// let mut cursor = Vehicle::find().cursor_by(vehicle::Column::Id)?;
    /// cursor.after(10).first(20);
    /// # Ok(())
    /// # }
    /// ```
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn cursor_by<K>(
        self,
        order_columns: K,
    ) -> Result<Cursor<DisjointRow<D>, K::ValueType>, Error>
    where
        K: KeyOf<D::Base>,
    {
        let selector = self.into_selector()?;
        Ok(
            Cursor::new(selector.query, Name::new(D::Base::default()), order_columns)
                .with_boundary_casts::<<D::Base as EntityTrait>::Column>(),
        )
    }
}

/// A disjoint read's rows are decoded by [`DisjointRow`], so that is what its
/// cursor selects.
// [spec:pgorm:sem:disjoint.read+1]
impl<D: Disjoint> CursorTrait for SelectDisjoint<D> {
    type Selector = DisjointRow<D>;
}
