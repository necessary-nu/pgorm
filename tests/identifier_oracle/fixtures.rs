//! Entities whose names are only known at run time, for the registry's
//! pgorm sites.

/// An entity whose table and schema names are computed per instance, the
/// pattern `EntityName::table_name(&self)` / `schema_name(&self)` exist for.
/// The name is leaked to `&'static str` so the entity stays `Copy`, which the
/// entity traits require; the oracle renders a few thousand statements, so
/// the leak is bounded.
pub mod dyn_named {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    pub struct Entity {
        pub table: &'static str,
        pub schema: Option<&'static str>,
    }

    impl Entity {
        /// The entity with `name` as its table.
        pub fn table(name: &str) -> Self {
            Self {
                table: leak(name),
                schema: None,
            }
        }

        /// The entity `t`, with `name` as its schema.
        pub fn schema(name: &str) -> Self {
            Self {
                table: "t",
                schema: Some(leak(name)),
            }
        }
    }

    fn leak(name: &str) -> &'static str {
        Box::leak(name.to_owned().into_boxed_str())
    }

    impl EntityName for Entity {
        fn table_name(&self) -> &str {
            self.table
        }

        fn schema_name(&self) -> Option<&str> {
            self.schema
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Text, indexed)]
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// An entity whose column is read through `select_with`, casting it to a type
/// named at run time — the read cast `select_sources` projects through.
pub mod cast_named {
    use std::cell::RefCell;

    use pgorm::{
        entity::prelude::*,
        pgorm_query::{Name, SimpleExpr},
    };

    thread_local! {
        static TYPE: RefCell<String> = RefCell::new(String::from("text"));
    }

    /// Run `f` with the column's read cast naming `name`.
    pub fn with_type<T>(name: &str, f: impl FnOnce() -> T) -> T {
        TYPE.with(|cell| cell.replace(name.to_owned()));
        let out = f();
        TYPE.with(|cell| cell.replace(String::from("text")));
        out
    }

    fn named_cast(expr: Expr) -> SimpleExpr {
        expr.cast_as(TYPE.with(|cell| Name::runtime(cell.borrow().clone())))
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "t")]
    pub struct Model {
        #[pgorm(primary_key, select_with = named_cast)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// An entity whose one column is of a range type a schema created, or of the
/// multirange beside one, named at run time through its `column_type` — the
/// cast `ColumnTrait::save_as` writes a value through.
pub mod range_named {
    use std::cell::RefCell;

    use pgorm::{entity::prelude::*, pgorm_query::Name};

    thread_local! {
        static TYPE: RefCell<(bool, Option<String>, String)> =
            RefCell::new((false, None, String::from("ty")));
    }

    /// Run `f` with the column's range type — its multirange when
    /// `multirange` — named `name` under `schema`.
    pub fn with_type<T>(
        multirange: bool,
        schema: Option<&str>,
        name: &str,
        f: impl FnOnce() -> T,
    ) -> T {
        TYPE.with(|cell| cell.replace((multirange, schema.map(str::to_owned), name.to_owned())));
        let out = f();
        TYPE.with(|cell| cell.replace((false, None, String::from("ty"))));
        out
    }

    fn named_range() -> ColumnType {
        let (multirange, schema, name) = TYPE.with(|cell| cell.borrow().clone());
        let (name, schema, subtype) = (
            Name::runtime(name),
            schema.map(Name::runtime),
            Arc::new(ColumnType::Integer),
        );
        match multirange {
            false => ColumnType::CreatedRange {
                name,
                schema,
                subtype,
            },
            true => ColumnType::CreatedMultirange {
                name,
                schema,
                subtype,
            },
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "t")]
    pub struct Model {
        #[pgorm(primary_key, column_type = named_range())]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A disjoint entity described at run time, as a language binding describes
/// one ([`DisjointShape::new`](pgorm::DisjointShape::new)), with the name
/// under test in one place and fixed names everywhere else: the base `t`,
/// keyed by `id`, holding its kind in `k` of the enum `kt`; the kind `a`,
/// with the table `ta` keyed by `id`; and the unit kind `b`. The schema,
/// each change and each statement are picked by index, so a site renders one
/// statement.
pub mod disjoint_named {
    use pgorm::{
        ColumnTypeTrait, DdlStatement, DisjointChange, DisjointColumn, DisjointShape,
        DisjointTable, Error,
        pgorm_query::{ColumnType, Name, TableName, TypeName},
    };

    use crate::identifier_oracle::oracle::Rendered;

    /// Where the name under test stands.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Slot {
        Base,
        KindTable,
        /// The key column of the base and of the kind's table.
        Key,
        KindColumn,
        KindType,
        TypeSchema,
        Value,
        /// The value [`remove_kind`] removes, which the shape does not declare.
        Removed,
        /// The table [`remove_kind_table`] drops.
        Dropped,
    }

    fn shape(slot: Slot, name: &str, tabled: bool) -> Result<DisjointShape, Error> {
        let at = |here: Slot, fixed: &str| {
            if slot == here {
                name.to_owned()
            } else {
                fixed.to_owned()
            }
        };
        let (key, kind, value) = (
            at(Slot::Key, "id"),
            at(Slot::KindColumn, "k"),
            value(slot, name),
        );
        let schema = (slot == Slot::TypeSchema).then(|| Name::runtime(name));
        let type_name = Name::runtime(at(Slot::KindType, "kt"));
        let kind_type = ColumnType::Enum {
            name: type_name.clone(),
            schema: schema.clone(),
            variants: vec![Name::runtime(value.clone()), Name::runtime("b")],
        };
        let mut kind_type_name = TypeName::new(type_name);
        kind_type_name.schema = schema;
        let table = |name: String, columns: Vec<DisjointColumn>| {
            DisjointTable::new(TableName::Table(Name::runtime(name)), columns, &[&key])
        };
        let base = table(
            at(Slot::Base, "t"),
            vec![
                DisjointColumn::new(&key, ColumnType::Integer.def()),
                DisjointColumn::new(&kind, kind_type.def()),
            ],
        )?;
        let kind_table = match tabled {
            true => Some(table(
                at(Slot::KindTable, "ta"),
                vec![DisjointColumn::new(&key, ColumnType::Integer.def())],
            )?),
            false => None,
        };
        DisjointShape::new(
            base,
            &kind,
            kind_type_name,
            vec![(value, kind_table), ("b".to_owned(), None)],
        )
    }

    fn value(slot: Slot, name: &str) -> String {
        match slot {
            Slot::Value => name.to_owned(),
            _ => "a".to_owned(),
        }
    }

    fn pick(statements: Result<Vec<DdlStatement>, Error>, index: usize) -> Rendered {
        match statements {
            Ok(statements) => match statements.get(index) {
                Some(statement) => Rendered::sql(statement.to_string()),
                None => Rendered::Refused(format!("no statement {index} in {statements:?}")),
            },
            Err(error) => Rendered::Refused(format!("{error:?}")),
        }
    }

    fn step(change: Result<DisjointChange, Error>, step: usize, index: usize) -> Rendered {
        pick(
            change.and_then(|change| {
                change.into_steps().into_iter().nth(step).map_or_else(
                    || Err(Error::Type(format!("no step {step}"))),
                    |step| Ok(step.statements().to_vec()),
                )
            }),
            index,
        )
    }

    /// [`DisjointShape::create`]'s `index`th statement.
    pub fn create(slot: Slot, name: &str, index: usize) -> Rendered {
        pick(
            shape(slot, name, true).and_then(|shape| shape.create()),
            index,
        )
    }

    pub fn add_kind(slot: Slot, name: &str, at: (usize, usize)) -> Rendered {
        let change = shape(slot, name, true).and_then(|shape| shape.add_kind(&value(slot, name)));
        step(change, at.0, at.1)
    }

    pub fn add_kind_recreating(slot: Slot, name: &str, index: usize) -> Rendered {
        let change =
            shape(slot, name, true).and_then(|shape| shape.add_kind_recreating(&value(slot, name)));
        step(change, 0, index)
    }

    pub fn remove_kind(slot: Slot, name: &str, index: usize) -> Rendered {
        let removed = match slot {
            Slot::Removed => name,
            _ => "z",
        };
        step(
            shape(slot, name, true).and_then(|shape| shape.remove_kind(removed)),
            0,
            index,
        )
    }

    pub fn add_kind_table(slot: Slot, name: &str, at: (usize, usize)) -> Rendered {
        let change =
            shape(slot, name, true).and_then(|shape| shape.add_kind_table(&value(slot, name)));
        step(change, at.0, at.1)
    }

    pub fn remove_kind_table(slot: Slot, name: &str, index: usize) -> Rendered {
        let dropped = match slot {
            Slot::Dropped => name,
            _ => "ta",
        };
        let change = shape(slot, name, false).and_then(|shape| {
            shape.remove_kind_table(&value(slot, name), TableName::Table(Name::runtime(dropped)))
        });
        step(change, 0, index)
    }
}

/// A [`OneOf`](pgorm::OneOf) reference over an entity whose table and
/// schema names are computed per instance from a thread-local, since
/// `Schema::create_one_of` names its table through the entity's `Default`.
pub mod one_of_named {
    use std::cell::RefCell;

    use pgorm::entity::prelude::*;
    use pgorm::tests_cfg::{photo, post};

    thread_local! {
        static NAMES: RefCell<(&'static str, Option<&'static str>)> =
            const { RefCell::new(("t", None)) };
    }

    /// Run `f` with the entity's table named `table` under `schema`.
    pub fn with_names<T>(table: &str, schema: Option<&str>, f: impl FnOnce() -> T) -> T {
        let leak = |name: &str| -> &'static str { Box::leak(name.to_owned().into_boxed_str()) };
        NAMES.with(|cell| cell.replace((leak(table), schema.map(leak))));
        let out = f();
        NAMES.with(|cell| cell.replace(("t", None)));
        out
    }

    #[derive(Copy, Clone, Debug, EntityTrait)]
    pub struct Entity {
        pub table: &'static str,
        pub schema: Option<&'static str>,
    }

    impl Default for Entity {
        fn default() -> Self {
            let (table, schema) = NAMES.with(|cell| *cell.borrow());
            Self { table, schema }
        }
    }

    impl EntityName for Entity {
        fn table_name(&self) -> &str {
            self.table
        }

        fn schema_name(&self) -> Option<&str> {
            self.schema
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub post_id: Option<i32>,
        pub photo_id: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(belongs_to = photo::Entity, from = Column::PhotoId, to = photo::Column::Id)]
        Photo,
        #[pgorm(belongs_to = post::Entity, from = Column::PostId, to = post::Column::Id)]
        Post,
    }

    /// What a row points at: a post or a photo, never neither.
    #[derive(Clone, Copy, Debug, OneOf)]
    #[pgorm(entity = Entity)]
    pub enum Pinned {
        #[pgorm(column = Column::PostId)]
        Post(post::Entity),
        #[pgorm(column = Column::PhotoId)]
        Photo(photo::Entity),
    }
}
