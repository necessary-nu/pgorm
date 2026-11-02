use crate::entity::compared_enum;
use crate::{ColumnTrait, EntityTrait};
use pgorm_query::{IntoName, Name, TypeName};

pub use pgorm_query::{IntoBoundary, IntoKey, Key};

/// The columns a relation joins on, held as `(from, to)` pairs so the two
/// sides of the join cannot disagree in length.
///
/// The only constructor takes the first pair, and every extension takes a pair,
/// so a set of join columns is non-empty and balanced by construction: there is
/// no unbalanced value to build, pass around, or truncate.
///
/// Beside the pairs matched for equality a set may hold one `PERIOD` pair,
/// PostgreSQL 18's temporal foreign key: the two sides' periods, a range or
/// multirange each, matched for overlap rather than equality. It is a slot of
/// its own, never one of the pairs, so it always comes after them.
///
/// A pair named by entity columns, as [`RelationBuilder`](crate::RelationBuilder)
/// names them, also records the enum each side compares as when its column's
/// domain is over one, which a pair named by bare names cannot know.
// [spec:pgorm:def:entity.relation.def+12]
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
#[derive(Debug, Clone)]
pub struct ColumnPairs {
    first: (Name, Name),
    rest: Vec<(Name, Name)>,
    period: Option<(Name, Name)>,
    compared: Vec<ComparedPair>,
}

/// The enums one pair's `from` and `to` sides compare as, each `None` unless
/// that side's column is typed with a domain over an enum.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
#[derive(Debug, Clone, Default)]
pub(crate) struct ComparedPair {
    pub(crate) from: Option<TypeName>,
    pub(crate) to: Option<TypeName>,
}

impl ComparedPair {
    fn of<F: ColumnTrait, T: ColumnTrait>(from: &F, to: &T) -> Self {
        Self {
            from: compared_enum(from.def().get_column_type()),
            to: compared_enum(to.def().get_column_type()),
        }
    }
}

impl ColumnPairs {
    /// Start a set of join columns from its first `(from, to)` pair.
    pub fn new<F, T>(from: F, to: T) -> Self
    where
        F: IntoName,
        T: IntoName,
    {
        Self {
            first: (from.into_name(), to.into_name()),
            rest: Vec::new(),
            period: None,
            compared: vec![ComparedPair::default()],
        }
    }

    /// [`ColumnPairs::new`] over entity columns, recording the enum each side
    /// compares as.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+14]
    pub(crate) fn of_columns<F, T>(from: F, to: T) -> Self
    where
        F: ColumnTrait,
        T: ColumnTrait,
    {
        let compared = ComparedPair::of(&from, &to);
        let mut pairs = Self::new(from, to);
        pairs.compared = vec![compared];
        pairs
    }

    /// [`ColumnPairs::push`] over entity columns, recording the enum each
    /// side compares as.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+14]
    pub(crate) fn push_columns<F, T>(&mut self, from: F, to: T)
    where
        F: ColumnTrait,
        T: ColumnTrait,
    {
        let compared = ComparedPair::of(&from, &to);
        self.rest.push((from.into_name(), to.into_name()));
        self.compared.push(compared);
    }

    /// Extend with a further pair, as a composite key requires.
    #[must_use]
    pub fn and<F, T>(mut self, from: F, to: T) -> Self
    where
        F: IntoName,
        T: IntoName,
    {
        self.push(from, to);
        self
    }

    /// Append a pair in place.
    pub fn push<F, T>(&mut self, from: F, to: T)
    where
        F: IntoName,
        T: IntoName,
    {
        self.rest.push((from.into_name(), to.into_name()));
        self.compared.push(ComparedPair::default());
    }

    /// Match `from` to `to` as periods, replacing any period pair already
    /// set: the relation joins a row to each row whose period overlaps its own.
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn set_period<F, T>(&mut self, from: F, to: T)
    where
        F: IntoName,
        T: IntoName,
    {
        self.period = Some((from.into_name(), to.into_name()));
    }

    /// The `(from, to)` period pair, if the relation is temporal.
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn period(&self) -> Option<&(Name, Name)> {
        self.period.as_ref()
    }

    /// Iterate the pairs matched for equality in declaration order; the
    /// period pair is not among them.
    pub fn iter(&self) -> impl Iterator<Item = &(Name, Name)> {
        std::iter::once(&self.first).chain(self.rest.iter())
    }

    /// The pairs matched for equality in declaration order, each beside the
    /// enums its sides compare as.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+14]
    pub(crate) fn compared_pairs(&self) -> impl Iterator<Item = (&(Name, Name), &ComparedPair)> {
        self.iter().zip(&self.compared)
    }

    /// The first pair, which every set has.
    pub fn first(&self) -> &(Name, Name) {
        &self.first
    }

    /// The number of pairs matched for equality, which is at least one.
    pub fn arity(&self) -> usize {
        1 + self.rest.len()
    }

    /// Swap every pair, the period pair with them, so the relation reads in
    /// the opposite direction.
    #[must_use]
    pub fn rev(self) -> Self {
        Self {
            first: (self.first.1, self.first.0),
            rest: self.rest.into_iter().map(|(f, t)| (t, f)).collect(),
            period: self.period.map(|(f, t)| (t, f)),
            compared: self
                .compared
                .into_iter()
                .map(|pair| ComparedPair {
                    from: pair.to,
                    to: pair.from,
                })
                .collect(),
        }
    }

    /// The `from` side of every pair, then the `from` period, as a [`Key`]:
    /// the columns that identify which source rows a join reached.
    pub fn from_key(&self) -> Key {
        self.side_key(|pair| Name::clone(&pair.0))
    }

    /// The `to` side of every pair, then the `to` period, as a [`Key`].
    pub fn to_key(&self) -> Key {
        self.side_key(|pair| Name::clone(&pair.1))
    }

    fn side_key<F>(&self, col: F) -> Key
    where
        F: Fn(&(Name, Name)) -> Name,
    {
        self.iter().chain(self.period.iter()).map(col).collect()
    }
}

impl IntoIterator for ColumnPairs {
    type Item = (Name, Name);
    type IntoIter = std::iter::Chain<std::iter::Once<Self::Item>, std::vec::IntoIter<Self::Item>>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(self.first).chain(self.rest)
    }
}

/// Check the [Key] of an Entity
pub trait KeyOf<E>: IntoKey
where
    E: EntityTrait,
{
    /// Method to call to perform this check
    fn key_of(self) -> Key;
}

impl<E, C> KeyOf<E> for C
where
    E: EntityTrait<Column = C>,
    C: ColumnTrait,
{
    fn key_of(self) -> Key {
        self.into_key()
    }
}

macro_rules! impl_key_of {
    ( $($T:ident),+ $(,)? ) => {
        impl<E, C> KeyOf<E> for ( $($T),+ )
        where
            E: EntityTrait<Column = C>,
            C: ColumnTrait,
        {
            fn key_of(self) -> Key {
                self.into_key()
            }
        }
    };
}

#[rustfmt::skip]
mod impl_key_of {
    use super::*;

    impl_key_of!(C, C);
    impl_key_of!(C, C, C);
    impl_key_of!(C, C, C, C);
    impl_key_of!(C, C, C, C, C);
    impl_key_of!(C, C, C, C, C, C);
    impl_key_of!(C, C, C, C, C, C, C);
    impl_key_of!(C, C, C, C, C, C, C, C);
    impl_key_of!(C, C, C, C, C, C, C, C, C);
    impl_key_of!(C, C, C, C, C, C, C, C, C, C);
    impl_key_of!(C, C, C, C, C, C, C, C, C, C, C);
    impl_key_of!(C, C, C, C, C, C, C, C, C, C, C, C);
}
