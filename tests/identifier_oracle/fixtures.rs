//! Entities whose names are only known at run time, for the registry's
//! pgorm sites.

/// An entity whose table and schema names are computed per instance, the
/// pattern `EntityName::table_name(&self)` / `schema_name(&self)` exist for.
/// The name is leaked to `&'static str` so the entity stays `Copy`, which the
/// entity traits require; the oracle renders a few thousand statements, so
/// the leak is bounded.
pub mod dyn_named {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, DeriveEntity)]
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

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Text, indexed)]
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
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

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "t")]
    pub struct Model {
        #[pgorm(primary_key, select_with = named_cast)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
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

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "t")]
    pub struct Model {
        #[pgorm(primary_key, column_type = named_range())]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
