//! Derive macros for pgorm.
//!
//! Every derive here expands to call-site-relative `pgorm::...` paths, never
//! `::pgorm::...`, so the name `pgorm` has to resolve wherever a derive is
//! written. Renaming the dependency in `Cargo.toml`
//! (`my_orm = { package = "pgorm", .. }`) breaks the expansions with a loud
//! `E0433`; alias it back with `use my_orm as pgorm;` in the module holding the
//! entities, or at the crate root. Relative paths are also what lets pgorm's own
//! test fixtures write `use crate as pgorm;`. `pgorm-query` needs no alias — the
//! expansions reach it through the `pgorm::pgorm_query` re-export — while
//! `pgorm-migration`, which `DeriveMigrationName` names directly, follows the
//! same rule as `pgorm`. There is no `#[pgorm(crate = ...)]` override.

extern crate proc_macro;

use proc_macro::TokenStream;

use syn::{DeriveInput, Error, parse_macro_input};

// [spec:pgorm:def:macros.derive+4]
#[cfg(feature = "derive")]
mod derives;

#[cfg(feature = "strum")]
mod strum;

/// Implement `EntityTrait` on an entity's own `Entity` struct, for an entity
/// whose table or schema name is computed at run time. The struct carries what
/// the name is computed from, `EntityName` is implemented on it by hand, and
/// the composite derive on the `Model` is given no `table_name`, so it writes
/// every other part of the entity.
///
/// ### Usage
///
/// ```
/// use pgorm::entity::prelude::*;
///
/// #[derive(Copy, Clone, Default, Debug, DeriveEntity)]
/// pub struct Entity {
///     pub tenant: &'static str,
/// }
///
/// impl EntityName for Entity {
///     fn schema_name(&self) -> Option<&str> {
///         Some(self.tenant)
///     }
///
///     fn table_name(&self) -> &str {
///         "cake"
///     }
/// }
///
/// #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub name: String,
/// }
///
/// #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// pub enum Relation {}
///
/// impl ActiveModelBehavior for ActiveModel {}
///
/// let cake = Entity { tenant: "bakery" };
/// assert_eq!(cake.schema_name(), Some("bakery"));
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveEntity, attributes(pgorm))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_derive_entity(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// This derive macro is the 'almighty' macro which automatically generates
/// Entity, Column, and PrimaryKey from a given Model.
///
/// ### Usage
///
/// ```
/// use pgorm::entity::prelude::*;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Deserialize, Serialize)]
/// #[pgorm(table_name = "posts")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub title: String,
///     #[pgorm(column_type = ColumnType::Text)]
///     pub text: String,
/// }
///
/// # #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// # pub enum Relation {}
/// #
/// # impl ActiveModelBehavior for ActiveModel {}
/// ```
///
/// Entity should always have a primary key.
/// Or, it will result in a compile error.
/// See <https://github.com/pgorm-rs/pgorm/issues/485> for details.
///
/// ```compile_fail
/// use pgorm::entity::prelude::*;
///
/// #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
/// #[pgorm(table_name = "posts")]
/// pub struct Model {
///     pub title: String,
///     #[pgorm(column_type = ColumnType::Text)]
///     pub text: String,
/// }
///
/// # #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// # pub enum Relation {}
/// #
/// # impl ActiveModelBehavior for ActiveModel {}
/// ```
///
/// The expansion writes call-site-relative `pgorm::...` paths, so the name `pgorm` has
/// to resolve in the module holding the entities. If the dependency is renamed in
/// `Cargo.toml` (`my_orm = { package = "pgorm", .. }`), alias it back with
/// `use my_orm as pgorm;` there or at the crate root; otherwise every generated path is
/// an `E0433`. There is no `#[pgorm(crate = ...)]` override.
// [spec:pgorm:def:macros.derive+4]
// [spec:pgorm:sem:macros.derive.entity-model+6]
// [spec:pgorm:req:macros.derive.entity-model.reject+2]
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveEntityModel, attributes(pgorm))]
pub fn derive_entity_model(input: TokenStream) -> TokenStream {
    let input_ts = input.clone();
    let DeriveInput {
        ident, data, attrs, ..
    } = parse_macro_input!(input as DeriveInput);

    if ident != "Model" {
        return Error::new(
            ident.span(),
            "DeriveEntityModel requires the struct to be named `Model`",
        )
        .into_compile_error()
        .into();
    }

    let mut ts: TokenStream = derives::expand_derive_entity_model(&ident, data.clone(), attrs)
        .unwrap_or_else(Error::into_compile_error)
        .into();
    ts.extend([
        derive_model(input_ts),
        derives::expand_active_model(ident, data)
            .unwrap_or_else(Error::into_compile_error)
            .into(),
    ]);
    ts
}

/// The DeriveColumn derive macro will implement `ColumnTrait` for Columns.
/// It defines the identifier of each column by implementing SqlName and StaticName.
/// The EnumIter is also derived, allowing iteration over all enum variants.
///
/// ### Usage
///
/// ```
/// use pgorm::entity::prelude::*;
///
/// #[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
/// pub enum Column {
///     CakeId,
///     FillingId,
/// }
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveColumn, attributes(pgorm))]
pub fn derive_column(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, data, .. } = parse_macro_input!(input);

    match derives::expand_derive_column(&ident, &data) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Implement `ModelTrait` and `FromQueryResult` for a further model of an
/// entity, beside the `Model` its composite derive makes: a struct of some of
/// the entity's columns, read from a row and through `ModelTrait` as `Model`
/// is. `#[pgorm(entity = ..)]` names the entity when it is not `Entity`.
///
/// ### Usage
///
/// ```
/// use pgorm::entity::prelude::*;
///
/// #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
/// #[pgorm(table_name = "cake")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub name: String,
/// }
///
/// #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// pub enum Relation {}
///
/// impl ActiveModelBehavior for ActiveModel {}
///
/// #[derive(Clone, Debug, PartialEq, DeriveModel)]
/// pub struct NameOnly {
///     pub name: String,
/// }
///
/// let named = NameOnly { name: "chiffon".to_owned() };
/// assert_eq!(named.get(Column::Name), "chiffon".into());
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveModel, attributes(pgorm))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_derive_model(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Derive into an active model
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveIntoActiveModel, attributes(pgorm))]
pub fn derive_into_active_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_into_active_model(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Implement `ActiveModelBehavior` for the `ActiveModel` with no hooks,
/// whatever the item it is written on.
///
/// ### Usage
///
/// ```
/// use pgorm::entity::prelude::*;
///
/// #[derive(Clone, Debug, PartialEq, DeriveEntityModel, DeriveActiveModelBehavior)]
/// #[pgorm(table_name = "cake")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub name: String,
/// }
///
/// #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// pub enum Relation {}
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveActiveModelBehavior)]
pub fn derive_active_model_behavior(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, data, .. } = parse_macro_input!(input);

    match derives::expand_derive_active_model_behavior(ident, data) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// A derive macro to implement `pgorm::ActiveEnum` trait for enums.
///
/// # Limitations
///
/// This derive macros can only be used on enums.
///
/// # Macro Attributes
///
/// All macro attributes listed below have to be annotated in the form of `#[pgorm(attr = value)]`.
///
/// - For enum
///     - `rs_type`: Define `ActiveEnum::Value`
///         - Possible values: `String`, `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`
///         - Note that value has to be passed as string, i.e. `rs_type = "i8"`
///     - `db_type`: Define `ColumnType` returned by `ActiveEnum::db_type()`
///         - Possible values: all available enum variants of `ColumnType`, e.g. `String(None)`, `String(Some(1))`, `Integer`
///         - Note that value has to be passed as string, i.e. `db_type = "Integer"`
///     - `enum_name`: Define `String` returned by `ActiveEnum::name()`
///         - This attribute is optional with default value being the name of enum in camel-case
///         - Note that value has to be passed as string, i.e. `db_type = "Integer"`
///
/// - For enum variant
///     - `string_value` or `num_value`:
///         - For `string_value`, value should be passed as string, i.e. `string_value = "A"`
///             - Due to the way internal Enums are automatically derived, the following restrictions apply:
///                 - members cannot share identical `string_value`, case-insensitive.
///                 - in principle, any future Titlecased Rust keywords are not valid `string_value`.
///         - For `num_value`, value should be passed as integer, i.e. `num_value = 1` or `num_value = 1i32`
///         - Note that only one of it can be specified, and all variants of an enum have to annotate with the same `*_value` macro attribute
///
/// # Usage
///
/// ```
/// use pgorm::{entity::prelude::*, DeriveActiveEnum};
///
/// #[derive(EnumIter, DeriveActiveEnum)]
/// #[pgorm(rs_type = "i32", db_type = "Integer")]
/// pub enum Color {
///     Black = 0,
///     White = 1,
/// }
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveActiveEnum, attributes(pgorm))]
pub fn derive_active_enum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derives::expand_derive_active_enum(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Convert a query result into the corresponding Model.
///
/// ### Attributes
/// - `skip`: Will not try to pull this field from the query result. And set it to the default value of the type.
///
/// ### Usage
///
/// ```
/// use pgorm::{entity::prelude::*, FromQueryResult};
///
/// #[derive(Debug, FromQueryResult)]
/// struct SelectResult {
///     name: String,
///     num_of_fruits: i32,
///     #[pgorm(skip)]
///     skip_me: i32,
/// }
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(FromQueryResult, attributes(pgorm))]
pub fn derive_from_query_result(input: TokenStream) -> TokenStream {
    let DeriveInput {
        ident,
        data,
        generics,
        ..
    } = parse_macro_input!(input);

    match derives::expand_derive_from_query_result(ident, data, generics) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// The DeriveRelation derive macro will implement RelationTrait for Relation.
///
/// ### Usage
///
/// ```
/// # use pgorm::tests_cfg::fruit::Entity;
/// use pgorm::entity::prelude::*;
///
/// #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// pub enum Relation {
///     #[pgorm(
///         belongs_to = "pgorm::tests_cfg::cake::Entity",
///         from = "pgorm::tests_cfg::fruit::Column::CakeId",
///         to = "pgorm::tests_cfg::cake::Column::Id"
///     )]
///     Cake,
///     #[pgorm(
///         belongs_to = "pgorm::tests_cfg::cake::Entity",
///         from = "pgorm::tests_cfg::fruit::Column::CakeId",
///         to = "pgorm::tests_cfg::cake::Column::Id",
///         on_condition = r#"|_left, right| Expr::col((right, pgorm::tests_cfg::cake::Column::Name)).like("%chocolate%")"#
///     )]
///     ChocolateCake,
/// }
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveRelation, attributes(pgorm))]
pub fn derive_relation(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_derive_relation(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// The DeriveMigrationName derive macro will implement `pgorm_migration::MigrationName` for a migration.
///
/// ### Usage
///
/// ```ignore
/// #[derive(DeriveMigrationName)]
/// pub struct Migration;
/// ```
///
/// The derive macro above will provide following implementation,
/// given the file name is `m20220120_000001_create_post_table.rs`.
///
/// ```ignore
/// impl MigrationName for Migration {
///     fn name(&self) -> &str {
///         "m20220120_000001_create_post_table"
///     }
/// }
/// ```
///
/// The expansion names `pgorm_migration` by a call-site-relative path, so a renamed
/// `pgorm-migration` dependency has to be aliased back
/// (`use my_migration as pgorm_migration;`) in the module holding the migration or at
/// the crate root, exactly as `pgorm` itself does. There is no `#[pgorm(crate = ...)]`
/// override.
// [spec:pgorm:def:macros.derive+4]
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveMigrationName)]
pub fn derive_migration_name(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_derive_migration_name(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Store a `Serialize + Deserialize` type in a `json`/`jsonb` column.
///
/// The derive generates the `TryGetableFromJson` marker, `From<Self> for Value`,
/// `ValueType`, and `Nullable`, so the type binds and decodes like any other column
/// value.
///
/// # Panics
///
/// The generated `From<Self> for Value` panics when `serde_json::to_value` refuses the
/// value — a map with non-string keys, or a `Serialize` impl that returns an error.
/// `From` has no channel to report failure, and the alternative is binding SQL NULL for
/// a value that was never null: a confusing constraint violation against a `NOT NULL`
/// column, and silent data loss against a nullable one. The panic message names the
/// type and carries serde's own error.
#[cfg(feature = "derive")]
#[proc_macro_derive(FromJsonQueryResult)]
pub fn derive_from_json_query_result(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);

    match derives::expand_derive_from_json_query_result(ident) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// The DerivePartialModel derive macro will implement `pgorm::PartialModelTrait` for simplify partial model queries.
///
/// ## Usage
///
/// ```rust
/// use pgorm::{entity::prelude::*, pgorm_query::Expr, DerivePartialModel, FromQueryResult};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Deserialize, Serialize)]
/// #[pgorm(table_name = "posts")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub title: String,
///     #[pgorm(column_type = ColumnType::Text)]
///     pub text: String,
/// }
/// # #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// # pub enum Relation {}
/// #
/// # impl ActiveModelBehavior for ActiveModel {}
///
/// #[derive(Debug, FromQueryResult, DerivePartialModel)]
/// #[pgorm(entity = "Entity")]
/// struct SelectResult {
///     title: String,
///     #[pgorm(from_col = "text")]
///     content: String,
///     #[pgorm(from_expr = "Expr::val(1).add(1)")]
///     sum: i32,
/// }
/// ```
///
/// If all fields in the partial model is `from_expr`, the `entity` can be ignore.
/// ```
/// use pgorm::{entity::prelude::*, pgorm_query::Expr, DerivePartialModel, FromQueryResult};
///
/// #[derive(Debug, FromQueryResult, DerivePartialModel)]
/// struct SelectResult {
///     #[pgorm(from_expr = "Expr::val(1).add(1)")]
///     sum: i32,
/// }
/// ```
///
/// A field cannot have attributes `from_col` and `from_expr` at the same time.
/// Or, it will result in a compile error.
///
/// ```compile_fail
/// use pgorm::{entity::prelude::*, FromQueryResult, DerivePartialModel, pgorm_query::Expr};
///
/// #[derive(Debug, FromQueryResult, DerivePartialModel)]
/// #[pgorm(entity = "Entity")]
/// struct SelectResult {
///     #[pgorm(from_expr = "Expr::val(1).add(1)", from_col = "foo")]
///     sum: i32
/// }
/// ```
///
/// A partial model with no fields selects nothing, so it cannot return the
/// `QuerySelect::Projected` state `select_cols` promises. The empty
/// projection is a compile error rather than a query that renders
/// `SELECT  FROM "tbl"`.
///
/// ```compile_fail,E0308
/// use pgorm::{entity::prelude::*, FromQueryResult, DerivePartialModel};
///
/// #[derive(Debug, FromQueryResult, DerivePartialModel)]
/// #[pgorm(entity = "Entity")]
/// struct SelectNothing {}
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(DerivePartialModel, attributes(pgorm))]
pub fn derive_partial_model(input: TokenStream) -> TokenStream {
    let derive_input = parse_macro_input!(input);

    match derives::expand_derive_partial_model(derive_input) {
        Ok(token_stream) => token_stream.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// [spec:pgorm:sem:macros.derive.test-attr+1]
#[doc(hidden)]
#[cfg(feature = "derive")]
#[proc_macro_attribute]
pub fn test(_: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::ItemFn);

    let ret = &input.sig.output;
    let name = &input.sig.ident;
    let body = &input.block;
    let attrs = &input.attrs;

    quote::quote! (
        #[test]
        #(#attrs)*
        fn #name() #ret {
            let _ = ::tracing_subscriber::fmt()
                .with_max_level(::tracing::Level::DEBUG)
                .with_test_writer()
                .try_init();
            crate::block_on!(async { #body })
        }
    )
    .into()
}

/// Creates a new type that iterates of the variants of an enum.
///
/// Iterate over the variants of an Enum. Any additional data on your variants will be set to `Default::default()`.
/// The macro implements `strum::IntoEnumIterator` on your enum and creates a new type called `YourEnumIter` that is the iterator object.
/// You cannot derive `EnumIter` on any type with a lifetime bound (`<'a>`) because the iterator would surely
/// create [unbounded lifetimes](https://doc.rust-lang.org/nightly/nomicon/unbounded-lifetimes.html).
#[cfg(feature = "strum")]
#[proc_macro_derive(EnumIter, attributes(strum))]
pub fn enum_iter(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    strum::enum_iter::enum_iter_inner(&ast)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Implements traits for types that wrap a database value type.
///
/// This procedure macro implements `From<T> for Value`, `pgorm::TryGetTable`, and
/// `pgorm_query::ValueType` for the wrapper type `T`.
///
/// ## Usage
///
/// ```rust
/// use pgorm::DeriveValueType;
///
/// #[derive(DeriveValueType)]
/// struct MyString(String);
/// ```
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveValueType, attributes(pgorm))]
pub fn derive_value_type(input: TokenStream) -> TokenStream {
    let derive_input = parse_macro_input!(input as DeriveInput);
    match derives::expand_derive_value_type(derive_input) {
        Ok(token_stream) => token_stream.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Derive the traits a newtype needs to stand for a type `CREATE TYPE ... AS
/// RANGE` makes: over a [`Range<T>`] for the range type, or over a
/// `Multirange<T>` for the multirange PostgreSQL creates beside it. It
/// implements `pgorm::CreatedRange`, `ValueType`, `Nullable`, `TryGetable`,
/// and conversions into its `Value` — the text form — and from what it holds.
///
/// `range_name` or `multirange_name` names the type, and `schema_name`
/// qualifies it; each is a name, quoted where it needs to be.
///
/// ```rust
/// use pgorm::entity::prelude::*;
///
/// #[derive(Clone, Debug, PartialEq, DeriveCreatedRange)]
/// #[pgorm(range_name = "floatrange")]
/// pub struct FloatRange(pub Range<f64>);
///
/// #[derive(Clone, Debug, PartialEq, DeriveCreatedRange)]
/// #[pgorm(multirange_name = "floatmultirange")]
/// pub struct FloatMultirange(pub Multirange<f64>);
/// ```
///
/// The field is what the name says it is:
///
/// ```compile_fail
/// use pgorm::entity::prelude::*;
///
/// #[derive(DeriveCreatedRange)]
/// #[pgorm(multirange_name = "floatmultirange")]
/// pub struct FloatMultirange(pub Range<f64>);
/// ```
///
/// [`Range<T>`]: https://docs.rs/pgorm-query/latest/pgorm_query/enum.Range.html
// [spec:pgorm:sem:macros.derive.created-range+1]
#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveCreatedRange, attributes(pgorm))]
pub fn derive_created_range(input: TokenStream) -> TokenStream {
    let derive_input = parse_macro_input!(input as DeriveInput);
    match derives::expand_derive_created_range(derive_input) {
        Ok(token_stream) => token_stream.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

#[cfg(feature = "derive")]
#[proc_macro_derive(DeriveDisplay, attributes(pgorm))]
pub fn derive_active_enum_display(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derives::expand_derive_active_enum_display(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// The DeriveSqlName derive macro will implement `pgorm::pgorm_query::SqlName` for simplify SqlName implementation.
///
/// ## Usage
///
/// ```rust
/// use pgorm::{DeriveSqlName, SqlName};
///
/// #[derive(DeriveSqlName)]
/// pub enum MyClass {
///     Table, // this is a special case, which maps to the enum's name
///     Id,
///     #[pgorm(iden = "turtle")]
///     Title,
///     Text,
/// }
///
/// #[derive(DeriveSqlName)]
/// struct MyOther;
///
/// assert_eq!(MyClass::Table.to_string(), "my_class");
/// assert_eq!(MyClass::Id.to_string(), "id");
/// assert_eq!(MyClass::Title.to_string(), "turtle"); // renamed!
/// assert_eq!(MyClass::Text.to_string(), "text");
/// assert_eq!(MyOther.to_string(), "my_other");
/// ```
#[cfg(feature = "derive")]
#[doc(alias = "DeriveIden")]
#[proc_macro_derive(DeriveSqlName, attributes(pgorm))]
pub fn derive_sql_name(input: TokenStream) -> TokenStream {
    let derive_input = parse_macro_input!(input as DeriveInput);

    match derives::expand_derive_sql_name(derive_input) {
        Ok(token_stream) => token_stream.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
