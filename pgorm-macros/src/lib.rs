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
//! `pgorm-migration`, which the `MigrationName` derive names directly, follows
//! the same rule as `pgorm`. There is no `#[pgorm(crate = ...)]` override.
//!
//! Each derive is named for what it makes or implements, as Rust's own derives
//! are, so a derive often shares its name with a trait: the two live in
//! different namespaces, and `use pgorm::ActiveEnum;` brings both.

extern crate proc_macro;

use proc_macro::TokenStream;

use syn::{DeriveInput, Error, parse_macro_input};

// [spec:pgorm:def:macros.derive+5]
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
/// `#[pgorm(column = .., model = .., active_model = .., primary_key = ..,
/// relation = ..)]` names a part that is not called the usual name; any other
/// key, the composite derive's own included, is a compile error.
///
/// ### Usage
///
/// ```
/// use pgorm::entity::prelude::*;
///
/// #[derive(Copy, Clone, Default, Debug, EntityTrait)]
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
/// #[derive(Clone, Debug, PartialEq, Entity)]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub name: String,
/// }
///
/// #[derive(Copy, Clone, Debug, EnumIter, Relation)]
/// pub enum Relation {}
///
/// impl ActiveModelBehavior for ActiveModel {}
///
/// let cake = Entity { tenant: "bakery" };
/// assert_eq!(cake.schema_name(), Some("bakery"));
/// ```
// [spec:pgorm:req:macros.derive.names]
#[cfg(feature = "derive")]
#[doc(alias = "DeriveEntity")]
#[proc_macro_derive(EntityTrait, attributes(pgorm))]
pub fn derive_entity_trait(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_derive_entity(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// The composite derive: on an entity's `Model` struct, write the rest of the
/// entity — its `Entity`, `Column`, `PrimaryKey` and `ActiveModel`, and
/// `ModelTrait` and `FromQueryResult` for the `Model`.
///
/// ### Usage
///
/// ```
/// use pgorm::entity::prelude::*;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Debug, PartialEq, Entity, Deserialize, Serialize)]
/// #[pgorm(table_name = "posts")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub title: String,
///     #[pgorm(column_type = ColumnType::Text)]
///     pub text: String,
/// }
///
/// # #[derive(Copy, Clone, Debug, EnumIter, Relation)]
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
/// #[derive(Clone, Debug, PartialEq, Entity)]
/// #[pgorm(table_name = "posts")]
/// pub struct Model {
///     pub title: String,
///     #[pgorm(column_type = ColumnType::Text)]
///     pub text: String,
/// }
///
/// # #[derive(Copy, Clone, Debug, EnumIter, Relation)]
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
// [spec:pgorm:def:macros.derive+5]
// [spec:pgorm:sem:macros.derive.entity-model+7]
// [spec:pgorm:req:macros.derive.entity-model.reject+3]
// [spec:pgorm:req:macros.derive.names]
#[cfg(feature = "derive")]
#[doc(alias = "DeriveEntityModel")]
#[proc_macro_derive(Entity, attributes(pgorm))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input_ts = input.clone();
    let DeriveInput {
        ident, data, attrs, ..
    } = parse_macro_input!(input as DeriveInput);

    if ident != "Model" {
        return Error::new(
            ident.span(),
            "`Entity` is derived on the struct named `Model`",
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

/// Name the variants of a column enum, for a custom select: implement
/// `SqlName`, `StaticName` and `FromStr` for it, each variant the snake_case
/// of its name unless `#[pgorm(column_name = "..")]` says otherwise.
///
/// ### Usage
///
/// ```
/// use pgorm::entity::prelude::*;
///
/// #[derive(Copy, Clone, Debug, EnumIter, Column)]
/// pub enum Column {
///     CakeId,
///     FillingId,
/// }
/// ```
#[cfg(feature = "derive")]
#[doc(alias = "DeriveColumn")]
#[proc_macro_derive(Column, attributes(pgorm))]
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
/// #[derive(Clone, Debug, PartialEq, Entity)]
/// #[pgorm(table_name = "cake")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub name: String,
/// }
///
/// #[derive(Copy, Clone, Debug, EnumIter, Relation)]
/// pub enum Relation {}
///
/// impl ActiveModelBehavior for ActiveModel {}
///
/// #[derive(Clone, Debug, PartialEq, Model)]
/// pub struct NameOnly {
///     pub name: String,
/// }
///
/// let named = NameOnly { name: "chiffon".to_owned() };
/// assert_eq!(named.get(Column::Name), "chiffon".into());
/// ```
#[cfg(feature = "derive")]
#[doc(alias = "DeriveModel")]
#[proc_macro_derive(Model, attributes(pgorm))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_derive_model(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Implement `IntoActiveModel` for a struct of some of an entity's columns, a
/// form: each field becomes its column's `Set` value and every other column
/// stays `NotSet`. `#[pgorm(active_model = ..)]` names the active model when it
/// is not `ActiveModel`.
#[cfg(feature = "derive")]
#[doc(alias = "DeriveIntoActiveModel")]
#[proc_macro_derive(IntoActiveModel, attributes(pgorm))]
pub fn derive_into_active_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_into_active_model(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Implement `ActiveEnum` for an enum, and the conversions that make it a
/// column's value.
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
///     - `display`: also implement `std::fmt::Display`, writing each variant's
///       `display_value`, or else its name. Opt-in, so an enum can implement
///       `Display` by hand instead
///
/// - For enum variant
///     - `string_value` or `num_value`:
///         - For `string_value`, value should be passed as string, i.e. `string_value = "A"`
///             - Due to the way internal Enums are automatically derived, the following restrictions apply:
///                 - members cannot share identical `string_value`, case-insensitive.
///                 - in principle, any future Titlecased Rust keywords are not valid `string_value`.
///         - For `num_value`, value should be passed as integer, i.e. `num_value = 1` or `num_value = 1i32`
///         - Note that only one of it can be specified, and all variants of an enum have to annotate with the same `*_value` macro attribute
///     - `display_value`: the text `display` writes for the variant
///
/// # Usage
///
/// ```
/// use pgorm::entity::prelude::*;
///
/// #[derive(EnumIter, ActiveEnum)]
/// #[pgorm(rs_type = "i32", db_type = "Integer", display)]
/// pub enum Color {
///     Black = 0,
///     #[pgorm(display_value = "Snow")]
///     White = 1,
/// }
///
/// assert_eq!(Color::Black.to_string(), "Black");
/// assert_eq!(Color::White.to_string(), "Snow");
/// ```
// [spec:pgorm:req:macros.derive.names]
#[cfg(feature = "derive")]
#[doc(alias = "DeriveActiveEnum", alias = "DeriveDisplay")]
#[proc_macro_derive(ActiveEnum, attributes(pgorm))]
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

/// Implement `RelationTrait` for an entity's `Relation` enum, each variant one
/// relation its `#[pgorm(..)]` describes.
///
/// ### Usage
///
/// ```
/// # use pgorm::tests_cfg::fruit::Entity;
/// use pgorm::entity::prelude::*;
///
/// #[derive(Copy, Clone, Debug, EnumIter, Relation)]
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
#[doc(alias = "DeriveRelation")]
#[proc_macro_derive(Relation, attributes(pgorm))]
pub fn derive_relation(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derives::expand_derive_relation(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Implement `pgorm_migration::MigrationName` for a migration, naming it for
/// the file it is written in.
///
/// ### Usage
///
/// ```ignore
/// #[derive(MigrationName)]
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
// [spec:pgorm:def:macros.derive+5]
#[cfg(feature = "derive")]
#[doc(alias = "DeriveMigrationName")]
#[proc_macro_derive(MigrationName)]
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
// [spec:pgorm:req:macros.derive.names]
#[cfg(feature = "derive")]
#[doc(alias = "FromJsonQueryResult")]
#[proc_macro_derive(JsonValueType)]
pub fn derive_json_value_type(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);

    match derives::expand_derive_json_value_type(ident) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Implement `PartialModelTrait` for a struct of some of an entity's columns
/// and expressions, the projection a select reads it from.
///
/// ## Usage
///
/// ```rust
/// use pgorm::{entity::prelude::*, pgorm_query::Expr, PartialModel, FromQueryResult};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Debug, PartialEq, Entity, Deserialize, Serialize)]
/// #[pgorm(table_name = "posts")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub title: String,
///     #[pgorm(column_type = ColumnType::Text)]
///     pub text: String,
/// }
/// # #[derive(Copy, Clone, Debug, EnumIter, Relation)]
/// # pub enum Relation {}
/// #
/// # impl ActiveModelBehavior for ActiveModel {}
///
/// #[derive(Debug, FromQueryResult, PartialModel)]
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
/// use pgorm::{entity::prelude::*, pgorm_query::Expr, PartialModel, FromQueryResult};
///
/// #[derive(Debug, FromQueryResult, PartialModel)]
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
/// use pgorm::{entity::prelude::*, FromQueryResult, PartialModel, pgorm_query::Expr};
///
/// #[derive(Debug, FromQueryResult, PartialModel)]
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
/// use pgorm::{entity::prelude::*, FromQueryResult, PartialModel};
///
/// #[derive(Debug, FromQueryResult, PartialModel)]
/// #[pgorm(entity = "Entity")]
/// struct SelectNothing {}
/// ```
#[cfg(feature = "derive")]
#[doc(alias = "DerivePartialModel")]
#[proc_macro_derive(PartialModel, attributes(pgorm))]
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
/// use pgorm::ValueType;
///
/// #[derive(ValueType)]
/// struct MyString(String);
/// ```
#[cfg(feature = "derive")]
#[doc(alias = "DeriveValueType")]
#[proc_macro_derive(ValueType, attributes(pgorm))]
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
/// #[derive(Clone, Debug, PartialEq, CreatedRange)]
/// #[pgorm(range_name = "floatrange")]
/// pub struct FloatRange(pub Range<f64>);
///
/// #[derive(Clone, Debug, PartialEq, CreatedRange)]
/// #[pgorm(multirange_name = "floatmultirange")]
/// pub struct FloatMultirange(pub Multirange<f64>);
/// ```
///
/// The field is what the name says it is:
///
/// ```compile_fail
/// use pgorm::entity::prelude::*;
///
/// #[derive(CreatedRange)]
/// #[pgorm(multirange_name = "floatmultirange")]
/// pub struct FloatMultirange(pub Range<f64>);
/// ```
///
/// [`Range<T>`]: https://docs.rs/pgorm-query/latest/pgorm_query/enum.Range.html
// [spec:pgorm:sem:macros.derive.created-range+2]
#[cfg(feature = "derive")]
#[doc(alias = "DeriveCreatedRange")]
#[proc_macro_derive(CreatedRange, attributes(pgorm))]
pub fn derive_created_range(input: TokenStream) -> TokenStream {
    let derive_input = parse_macro_input!(input as DeriveInput);
    match derives::expand_derive_created_range(derive_input) {
        Ok(token_stream) => token_stream.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Implement `SqlName` for an enum or unit struct: pgorm-query's derive of the
/// same name, its expansion naming the trait through `pgorm::pgorm_query` so a
/// crate depending only on pgorm can use it.
///
/// A unit struct is the snake_case of its name; an enum's variants are each the
/// snake_case of theirs, and a variant named `Table` is the enum's own name.
/// `#[iden = ".."]` renames the type or a variant, `#[method = ".."]` names a
/// method a variant's name is read from, and `#[iden(flatten)]` takes the name
/// of a variant's one field.
///
/// ```rust
/// use pgorm::SqlName;
///
/// #[derive(SqlName)]
/// pub enum MyClass {
///     Table,
///     Id,
///     #[iden = "turtle"]
///     Title,
///     Text,
/// }
///
/// #[derive(SqlName)]
/// struct MyOther;
///
/// assert_eq!(MyClass::Table.to_string(), "my_class");
/// assert_eq!(MyClass::Id.to_string(), "id");
/// assert_eq!(MyClass::Title.to_string(), "turtle");
/// assert_eq!(MyClass::Text.to_string(), "text");
/// assert_eq!(MyOther.to_string(), "my_other");
/// ```
// [spec:pgorm:sem:macros.derive.iden+3]
// [spec:pgorm:req:macros.derive.names]
#[cfg(feature = "derive")]
#[doc(alias = "DeriveSqlName", alias = "DeriveIden", alias = "Iden")]
#[proc_macro_derive(SqlName, attributes(iden, method))]
pub fn derive_sql_name(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    pgorm_query_derive_internals::sql_name(&input, &quote::quote!(pgorm::pgorm_query))
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Implement `StaticName`, `SqlName` and `AsRef<str>` for an enum or unit
/// struct, named as [`SqlName`](derive@SqlName) names it: pgorm-query's derive
/// of the same name, its expansion naming the traits through
/// `pgorm::pgorm_query`.
///
/// ```rust
/// use pgorm::StaticName;
///
/// #[derive(Clone, Copy, Debug, StaticName)]
/// enum Shelf {
///     Table,
///     #[iden = "top_shelf"]
///     Top,
/// }
///
/// assert_eq!(Shelf::Table.as_str(), "shelf");
/// assert_eq!(Shelf::Top.as_str(), "top_shelf");
/// ```
// [spec:pgorm:sem:macros.derive.iden+3]
// [spec:pgorm:req:macros.derive.names]
#[cfg(feature = "derive")]
#[proc_macro_derive(StaticName, attributes(iden, method))]
pub fn derive_static_name(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    pgorm_query_derive_internals::static_name(&input, &quote::quote!(pgorm::pgorm_query))
        .unwrap_or_else(Error::into_compile_error)
        .into()
}
