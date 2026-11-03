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
use crate::entity::no_such_kind;
use crate::error::query_err;
use crate::query::BASE_SOURCE;
use crate::query::graph::source_prefix;
use crate::{
    ConnectionTrait, Cursor, CursorTrait, Disjoint, DisjointShape, EntityTrait, Error,
    FromQueryResult, Iterable, KeyOf, KindModel, KindVisitor, ModelTrait, Paginator,
    PaginatorTrait, PinBoxSendStream, PrimaryKeyToColumn, QueryResult, SelectDisjoint, Selector,
    SelectorRaw, SelectorTrait, StaticName, WriteDisjoint, check_disjoint, table_label,
};

/// The selector that decodes one row of a disjoint read: the base's model
/// under `s0_`, and the value of `D` its kind gives.
///
/// The row's kind is its base row's. A kind without a table is its unit
/// variant; a kind with one decodes the row joined from its table into its
/// variant. A row of a kind with a table whose joined row is absent, and a
/// row with a joined row present for any other kind, are each an error
/// naming the base table, the key and the kinds involved
/// ([`DisjointShape::judge`] judges them alike): neither is a state the
/// constraints `Schema::create_disjoint` writes admit, so neither is read as
/// a variant without its row, nor dropped.
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

/// One kind's part of a row: its value, and for a kind with a table, the
/// table, the decoded source it was projected as and whether its row is
/// there.
struct KindPart {
    value: String,
    table: Option<(String, usize, bool)>,
}

/// Judge one row's kinds: `own` is the row's kind as its kind column holds
/// it, or `None` for an unkinded entity, and `kinds` every kind's part in
/// declaration order. The row's kind is returned beside the source to decode
/// its row from, or `None` for a kind without a table. Anything the
/// declaration does not describe is the [`Error::Query`] naming the base
/// table `base`, the key `key` gives, each column as `` `column` = value ``
/// in key order, and the kinds involved.
///
/// A kinded row holds exactly its own kind's row, where its kind has a
/// table, and no other kind's. An unkinded row is of the one kind whose row
/// is present, or of the unit kind where none is.
// [spec:pgorm:sem:disjoint.read+1]
// [spec:pgorm:sem:disjoint.unkinded+1]
fn judge(
    base: &str,
    kinds: &[KindPart],
    own: Option<usize>,
    key: impl FnOnce() -> Vec<String>,
) -> Result<(usize, Option<usize>), Error> {
    let Some(own) = own else {
        return judge_presence(base, kinds, key);
    };
    let mut source = None;
    let mut missing = None;
    let mut strays = Vec::new();
    for (index, kind) in kinds.iter().enumerate() {
        match (&kind.table, index == own) {
            (Some((_, at, true)), true) => source = Some(*at),
            (Some((table, _, false)), true) => missing = Some(table),
            (Some((table, _, true)), false) => strays.push((&kind.value, table)),
            (Some((_, _, false)), false) | (None, _) => {}
        }
    }
    if missing.is_none() && strays.is_empty() {
        return Ok((own, source));
    }
    let mut faults = Vec::new();
    if let Some(table) = missing {
        faults.push(format!("no row in `{table}`, its kind's table"));
    }
    for (stray, table) in strays {
        faults.push(format!(
            "a row in `{table}`, the table of the kind `{stray}`"
        ));
    }
    Err(query_err(format!(
        "the `{base}` row whose key is ({key}) is of the kind `{kind}` but has {faults}: the \
         constraints `Schema::create_disjoint` writes hold each base row to exactly its own \
         kind's table, so this schema was not created by it",
        key = key().join(", "),
        kind = kinds
            .get(own)
            .map(|kind| kind.value.as_str())
            .unwrap_or_default(),
        faults = faults.join(" and "),
    )))
}

/// Judge an unkinded row by which of its kinds' rows are present: the one
/// present is its kind, and with none present it is its unit kind. None
/// present where it has no unit kind, and two or more present, are the
/// [`Error::Query`] [`judge`] describes.
// [spec:pgorm:sem:disjoint.unkinded+1]
fn judge_presence(
    base: &str,
    kinds: &[KindPart],
    key: impl FnOnce() -> Vec<String>,
) -> Result<(usize, Option<usize>), Error> {
    let present: Vec<(usize, &str, usize)> = kinds
        .iter()
        .enumerate()
        .filter_map(|(index, kind)| match &kind.table {
            Some((table, at, true)) => Some((index, table.as_str(), *at)),
            _ => None,
        })
        .collect();
    let faults = match present.as_slice() {
        [(index, _, at)] => return Ok((*index, Some(*at))),
        [] => {
            if let Some(unit) = kinds.iter().position(|kind| kind.table.is_none()) {
                return Ok((unit, None));
            }
            let tables: Vec<String> = kinds
                .iter()
                .filter_map(|kind| kind.table.as_ref())
                .map(|(table, ..)| format!("`{table}`"))
                .collect();
            format!(
                "a row in none of its kinds' tables, {}, and the entity declares no unit kind \
                 for such a row",
                tables.join(", ")
            )
        }
        many => {
            let rows: Vec<String> = many
                .iter()
                .map(|(index, table, _)| {
                    format!(
                        "a row in `{table}`, the table of the kind `{}`",
                        kinds[*index].value
                    )
                })
                .collect();
            format!(
                "{}: nothing in an unkinded entity's schema holds a base row to one kind's \
                 table, so its kind cannot be told",
                rows.join(" and ")
            )
        }
    };
    Err(query_err(format!(
        "the `{base}` row whose key is ({key}) of an unkinded disjoint entity has {faults}",
        key = key().join(", "),
    )))
}

/// The kind a row of a read of a [`DisjointShape`] is of, as
/// [`DisjointShape::judge`] judges it.
// [spec:pgorm:sem:disjoint.read+1]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisjointJudged {
    /// The index of the row's kind among the shape's kinds.
    pub kind: usize,
    /// The decoded source the kind's row is read from, as
    /// [`DisjointShape::sources`] numbers it, or `None` for a kind without a
    /// table.
    pub source: Option<usize>,
}

impl DisjointShape {
    /// Judge one row of a read of this shape: the kind its base row is of,
    /// read from its kind column, or, for an unkinded shape, from which of
    /// its kinds' rows is present, and where its kind's row is.
    ///
    /// A row of a kind with a table whose joined row is absent, and a row
    /// with a joined row present for another kind, are each the
    /// [`Error::Query`] a typed read reports, naming the base table, the key
    /// `key` gives each key column, in key order, and the kinds involved, as
    /// are an unkinded row with rows present in two kinds' tables and one
    /// with a row in none where the shape has no unit kind; a kind the shape
    /// lacks is an [`Error::Type`], and a kind column that does not read as
    /// text is the decode error reading it gives. A row's joined row is there
    /// unless every column of it is `NULL`, as the absence witness judges a
    /// typed one.
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    // [spec:pgorm:sem:disjoint.unkinded+1]
    pub fn judge(
        &self,
        row: &QueryResult,
        key: impl FnOnce() -> Vec<String>,
    ) -> Result<DisjointJudged, Error> {
        let base_prefix = source_prefix(BASE_SOURCE);
        let own = match self.kind_column() {
            Some(column) => {
                let label: String = row.try_get(&base_prefix, &column.name.to_string())?;
                Some(self.kind(&label)?.0)
            }
            None => None,
        };
        let mut sources = BASE_SOURCE;
        let parts: Vec<KindPart> = self
            .kinds
            .iter()
            .map(|entry| KindPart {
                value: entry.value.clone(),
                table: entry.table.as_ref().map(|table| {
                    sources += 1;
                    let names: Vec<String> = table
                        .columns
                        .iter()
                        .map(|column| column.name.to_string())
                        .collect();
                    let absent =
                        row.all_null(&source_prefix(sources), names.iter().map(String::as_str));
                    (table.label.clone(), sources, !absent)
                }),
            })
            .collect();
        let key = || {
            self.base
                .key()
                .zip(key())
                .map(|(column, value)| format!("`{}` = {value}", column.name.to_string()))
                .collect()
        };
        let (kind, source) = judge(&self.base.label, &parts, own, key)?;
        Ok(DisjointJudged { kind, source })
    }
}

// [spec:pgorm:sem:disjoint.read+1]
impl<D: Disjoint> SelectorTrait for DisjointRow<D> {
    type Item = DisjointItem<D>;

    fn from_raw_query_result(res: QueryResult) -> Result<Self::Item, Error> {
        let base_prefix = source_prefix(BASE_SOURCE);
        let base = <D::Base as EntityTrait>::Model::from_query_result(&res, &base_prefix)?;
        let label = match D::kind_column() {
            Some(kind_column) => {
                Some(res.try_get::<String>(&base_prefix, kind_column.column.as_str())?)
            }
            None => None,
        };

        let mut sources = BASE_SOURCE;
        let mut parts = Vec::new();
        for visited in D::Kind::iter() {
            parts.push(D::visit_kind(
                visited,
                Presence {
                    res: &res,
                    sources: &mut sources,
                },
            ));
        }
        let label_of_base = || table_label(&D::Base::default());
        let own = match label {
            Some(label) => Some(
                parts
                    .iter()
                    .position(|part| part.value == label)
                    .ok_or_else(|| {
                        no_such_kind(
                            &label_of_base(),
                            &label,
                            parts.iter().map(|part| part.value.as_str()),
                        )
                    })?,
            ),
            None => None,
        };
        let key = || {
            <D::Base as EntityTrait>::PrimaryKey::iter()
                .map(|key| {
                    let column = key.into_column();
                    format!("`{}` = {}", column.as_str(), base.get(column))
                })
                .collect()
        };
        let (own, source) = judge(&label_of_base(), &parts, own, key)?;
        let kind = D::Kind::iter()
            .nth(own)
            .ok_or_else(|| query_err("a row was judged of a kind its entity lacks".to_owned()))?;
        let value = D::visit_kind(kind, Decode { res: &res, source })?;
        Ok((base, value))
    }
}

/// Judges one kind's part of a row by the absence witness alone, so a row
/// present for a kind that is not the row's own is reported as present even
/// if it would not decode.
///
/// `sources` counts the kinds with a table visited so far, as the projection
/// counted them (the shape's projection over the same kinds in the same
/// order), so each kind is read under the prefix it was projected under.
struct Presence<'r> {
    res: &'r QueryResult,
    sources: &'r mut usize,
}

impl<D: Disjoint> KindVisitor<D> for Presence<'_> {
    type Output = KindPart;

    // [spec:pgorm:sem:disjoint.read+1]
    fn table<M: KindModel<D>>(self, kind: D::Kind) -> KindPart {
        *self.sources += 1;
        let present = !absent_row::<M>(self.res, &source_prefix(*self.sources));
        KindPart {
            value: D::kind_name(kind).to_owned(),
            table: Some((table_label(&M::Entity::default()), *self.sources, present)),
        }
    }

    fn unit(self, value: D) -> KindPart {
        KindPart {
            value: D::kind_name(value.kind()).to_owned(),
            table: None,
        }
    }
}

/// Decodes the row's own kind into its variant: from the source its row was
/// judged to be under, or as its unit variant.
struct Decode<'r> {
    res: &'r QueryResult,
    source: Option<usize>,
}

impl<D: Disjoint> KindVisitor<D> for Decode<'_> {
    type Output = Result<D, Error>;

    // [spec:pgorm:sem:disjoint.read+1]
    fn table<M: KindModel<D>>(self, _: D::Kind) -> Self::Output {
        let source = self
            .source
            .ok_or_else(|| query_err("a kind with a table was judged to have no row".to_owned()))?;
        M::from_query_result(self.res, &source_prefix(source)).map(M::into_variant)
    }

    fn unit(self, value: D) -> Self::Output {
        Ok(value)
    }
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
        let shape = self.shape.clone();
        let selector = self.into_selector()?;
        Ok(Cursor::new(
            selector.query,
            Name::clone(shape.base().name()),
            order_columns,
        )
        .with_casts_by(|column| shape.boundary_cast(column)))
    }
}

// [spec:pgorm:sem:disjoint.write+1]
impl<D: Disjoint> WriteDisjoint<D> {
    /// Send the write, one statement, and return the pair a read of the
    /// written row gives, decoded as a [`SelectDisjoint<D>`]'s row is.
    ///
    /// A write whose base part touches no row — an update or a change of
    /// kind whose key no row has, an update of a row of another kind, a
    /// change of kind to the row's own kind — returns no row and is
    /// [`Error::RecordNotFound`], having changed nothing. A constraint the
    /// write breaks is the statement's error, reported at its end, and
    /// nothing of it applies.
    // [spec:pgorm:sem:disjoint.write+1]
    pub async fn exec_returning_model<C: ConnectionTrait>(
        self,
        db: &C,
    ) -> Result<DisjointItem<D>, Error> {
        let (stmt, values) = self.query.build();
        SelectorRaw {
            stmt,
            values,
            selector: DisjointRow::<D>(PhantomData),
        }
        .one(db)
        .await
    }
}

/// A disjoint read's rows are decoded by [`DisjointRow`], so that is what its
/// cursor selects.
// [spec:pgorm:sem:disjoint.read+1]
impl<D: Disjoint> CursorTrait for SelectDisjoint<D> {
    type Selector = DisjointRow<D>;
}
