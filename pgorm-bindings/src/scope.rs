//! Scopes as both bindings hold a statement to them (`bindings.scopes`):
//! the context a program gives, read into a [`Context`]; a declared model's
//! scope columns, [`DeclaredScopes`], which decide what a statement under a
//! context is held to, what an insert fills and what each refuses; and
//! [`Scoping`], a context of Rust scope values through which a registered
//! entity's `_for` forms run.
//!
//! Each binding reads its language's context into a [`Context`] of its own
//! values and converts them as its columns convert a value; everything a
//! scope decides is decided here, once, and worded as pgorm words it.

use std::{
    any::{Any, TypeId},
    fmt,
    sync::{Arc, PoisonError, RwLock},
};

use pgorm::pgorm_query::{ConflictAction, ConflictAssignment, OnConflict, Value, ValueType};
use pgorm::{Held, Provides, Scope, ScopeContext};

use crate::{Dialect, Failure, Tag, entity::ColumnInfo};

/// How a context holds one scope: one value, which a statement under it is
/// held to, or every value, lifted on purpose, as Rust's `Every` is.
// [spec:pgorm:req:bindings.scopes]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holding<V> {
    /// One value: a statement's rows are this value's.
    One(V),
    /// Every value: the scope puts no condition on a statement, and an
    /// insert takes the column from its row.
    Every,
}

impl<V> Holding<V> {
    /// The holding of a reference to the value.
    pub fn as_ref(&self) -> Holding<&V> {
        match self {
            Self::One(value) => Holding::One(value),
            Self::Every => Holding::Every,
        }
    }
}

/// A context as a program gave it: the scopes it holds, each by its name,
/// and the name of its type, which a refusal names it by as Rust names a
/// context by its type.
// [spec:pgorm:req:bindings.scopes]
#[derive(Clone, Debug)]
pub struct Context<V> {
    label: String,
    held: Vec<(String, Holding<V>)>,
}

impl<V> Context<V> {
    /// The context `label` names, holding `held`: refused where it holds one
    /// scope twice, as Rust's `ScopeContext` derive refuses a struct that
    /// does.
    pub fn new(label: impl Into<String>, held: Vec<(String, Holding<V>)>) -> Result<Self, Failure> {
        let label = label.into();
        for (index, (name, _)) in held.iter().enumerate() {
            if held[..index].iter().any(|(seen, _)| seen == name) {
                return Err(Failure::Construction(format!(
                    "`{label}` holds `{name}` twice: a context holds each scope once, as one value or as every value"
                )));
            }
        }
        Ok(Self { label, held })
    }

    /// The name a refusal names the context by.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// How the context holds the scope `scope`, or `None` where it does not.
    pub fn holding(&self, scope: &str) -> Option<Holding<&V>> {
        self.held
            .iter()
            .find(|(name, _)| name == scope)
            .map(|(_, holding)| holding.as_ref())
    }

    /// Every scope the context holds, in the order it was given them.
    pub fn held(&self) -> impl Iterator<Item = (&str, Holding<&V>)> {
        self.held
            .iter()
            .map(|(name, holding)| (name.as_str(), holding.as_ref()))
    }

    /// The same context, each value converted by `convert`.
    pub fn try_map<W, E>(
        self,
        mut convert: impl FnMut(&str, V) -> Result<W, E>,
    ) -> Result<Context<W>, E> {
        let held = self
            .held
            .into_iter()
            .map(|(name, holding)| {
                let holding = match holding {
                    Holding::One(value) => Holding::One(convert(&name, value)?),
                    Holding::Every => Holding::Every,
                };
                Ok((name, holding))
            })
            .collect::<Result<_, E>>()?;
        Ok(Context {
            label: self.label,
            held,
        })
    }
}

/// `names` as a refusal lists them: `` `A` ``, `` `A` and `B` ``,
/// `` `A`, `B` and `C` ``.
fn listed(names: &[&str]) -> String {
    let names: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// The refusal of a statement on `table`, scoped by `scopes`, built under a
/// context `label` names that does not hold each of them, or under none:
/// where Rust's compile error stops the statement, in its words, the
/// context named by its type and a statement taking none told where its
/// context goes.
// [spec:pgorm:req:bindings.scopes]
pub fn unheld(table: &str, scopes: &[&str], label: Option<&str>, dialect: Dialect) -> Failure {
    let list = listed(scopes);
    Failure::Construction(match label {
        Some(label) => format!("`{table}` is scoped by {list}, which `{label}` does not hold"),
        None => format!(
            "`{table}` is scoped by {list}, which a statement taking no context does not hold: \
             a statement on `{table}` starts from a `{}` form, given a context holding its \
             scopes first: `{}(cx)`, `{}(cx, values)`, `{}(cx, values)`",
            dialect.option("_for"),
            dialect.option("find_for"),
            dialect.option("insert_for"),
            dialect.option("update_for"),
        ),
    })
}

/// Refuse a statement on `table`, scoped by `scopes`, built under `cx`,
/// unless `cx` holds each of them: `None` is a statement given no context.
// [spec:pgorm:req:bindings.scopes]
pub fn check<V>(
    table: &str,
    scopes: &[&str],
    cx: Option<&Context<V>>,
    dialect: Dialect,
) -> Result<(), Failure> {
    match cx {
        _ if scopes.is_empty() => Ok(()),
        Some(cx) if scopes.iter().all(|scope| cx.holding(scope).is_some()) => Ok(()),
        _ => Err(unheld(table, scopes, cx.map(Context::label), dialect)),
    }
}

/// The refusal of a join into `table`, scoped by `scopes`, by a form taking
/// no context, as Rust's `ScopeError::Unscoped` words it.
// [spec:pgorm:req:bindings.scopes]
pub fn unscoped_join(table: &str, scopes: &[&str], dialect: Dialect) -> Failure {
    Failure::Construction(format!(
        "`{table}` is scoped by {}; join it with a form taking the statement's context, such as `{}`",
        listed(scopes),
        dialect.option("join_for"),
    ))
}

/// What an insert under a context fills in its row, each field with the
/// context's value, or the refusal of the row.
pub type Filled<'a, W> = Result<Vec<(&'a str, W)>, Failure>;

/// One of a declared model's scope columns: the field, its column's SQL
/// name, and the name of the scope it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeColumn {
    /// The model's field.
    pub field: String,
    /// The column's SQL name.
    pub column: String,
    /// The scope the column holds, by its name.
    pub scope: String,
}

/// A declared model's scope columns, as the program declared them: what a
/// statement on its table under a context is held to.
// [spec:pgorm:req:bindings.scopes]
#[derive(Clone, Debug)]
pub struct DeclaredScopes {
    table: String,
    columns: Vec<ScopeColumn>,
}

impl DeclaredScopes {
    /// The scope columns `columns` of the table `table`: one field per
    /// scope, since a row is in one value of each.
    pub fn new(table: impl Into<String>, columns: Vec<ScopeColumn>) -> Result<Self, Failure> {
        let table = table.into();
        for (index, column) in columns.iter().enumerate() {
            if let Some(other) = columns[..index]
                .iter()
                .find(|seen| seen.scope == column.scope)
            {
                return Err(Failure::Construction(format!(
                    "`{table}`'s fields {} and {} both hold `{}`: a scope is one column of its table",
                    other.field, column.field, column.scope
                )));
            }
        }
        Ok(Self { table, columns })
    }

    /// The table, as a refusal names it.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// The scope columns, in declaration order.
    pub fn columns(&self) -> &[ScopeColumn] {
        &self.columns
    }

    /// The scopes, in declaration order.
    pub fn names(&self) -> Vec<&str> {
        self.columns
            .iter()
            .map(|column| column.scope.as_str())
            .collect()
    }

    /// How `cx` holds each scope column's scope, refused where `cx` is
    /// `None` or does not hold one: what every statement on the table built
    /// under it is held to.
    pub fn held<'c, V>(
        &self,
        cx: Option<&'c Context<V>>,
        dialect: Dialect,
    ) -> Result<Vec<(&ScopeColumn, Holding<&'c V>)>, Failure> {
        if self.columns.is_empty() {
            return Ok(Vec::new());
        }
        let refused = || unheld(&self.table, &self.names(), cx.map(Context::label), dialect);
        let cx = cx.ok_or_else(refused)?;
        self.columns
            .iter()
            .map(|column| {
                cx.holding(&column.scope)
                    .map(|holding| (column, holding))
                    .ok_or_else(refused)
            })
            .collect()
    }

    /// The fields a read, an update or a delete under `cx` is held to equal,
    /// each with the value: every scope column whose scope `cx` holds one
    /// value of, a scope under every value adding none.
    pub fn condition<'c, V>(
        &self,
        cx: Option<&'c Context<V>>,
        dialect: Dialect,
    ) -> Result<Vec<(&str, &'c V)>, Failure> {
        Ok(self
            .held(cx, dialect)?
            .into_iter()
            .filter_map(|(column, holding)| match holding {
                Holding::One(value) => Some((column.field.as_str(), value)),
                Holding::Every => None,
            })
            .collect())
    }

    /// The fields an insert under `cx` sets that its row, whose value of a
    /// field `row` gives, leaves unset, each with the value the context
    /// holds, as Rust's insert fills a row: a row setting a scope column to
    /// the context's value is kept, one setting another is refused, and
    /// under every value of a scope the row sets the column itself or is
    /// refused. `convert` makes a value of either side comparable, as its
    /// column converts a value it writes; its error is the outer one.
    pub fn fill<R, W: PartialEq, E>(
        &self,
        cx: Option<&Context<R>>,
        row: impl Fn(&str) -> Option<R>,
        mut convert: impl FnMut(&ScopeColumn, &R) -> Result<W, E>,
        dialect: Dialect,
    ) -> Result<Filled<'_, W>, E> {
        let held = match self.held(cx, dialect) {
            Ok(held) => held,
            Err(refused) => return Ok(Err(refused)),
        };
        let mut fills = Vec::new();
        for (column, holding) in held {
            match (holding, row(&column.field)) {
                (Holding::One(value), None) => {
                    fills.push((column.field.as_str(), convert(column, value)?));
                }
                (Holding::One(value), Some(set)) => {
                    if convert(column, &set)? != convert(column, value)? {
                        return Ok(Err(self.conflict(column)));
                    }
                }
                (Holding::Every, None) => {
                    return Ok(Err(Failure::Construction(format!(
                        "the context holds every `{}`, so the row sets `{}`.`{}` itself, and it is not set",
                        column.scope, self.table, column.column
                    ))));
                }
                (Holding::Every, Some(_)) => {}
            }
        }
        Ok(Ok(fills))
    }

    /// The refusal of a row setting `column` to a value outside its scope,
    /// as Rust's `ScopeError::Conflict` words it.
    pub fn conflict(&self, column: &ScopeColumn) -> Failure {
        Failure::Construction(format!(
            "the row sets `{}`.`{}` to a value outside the context's `{}`",
            self.table, column.column, column.scope
        ))
    }

    /// Refuse an upsert whose `ON CONFLICT .. DO UPDATE SET` assigns a
    /// scope column, as Rust refuses one, by the column's SQL name: an
    /// update of the row it conflicted with would move it out of its scope.
    pub fn upsert(&self, on_conflict: &OnConflict) -> Result<(), Failure> {
        let OnConflict::Targeted {
            action: ConflictAction::Update { sets, .. },
            ..
        } = on_conflict
        else {
            return Ok(());
        };
        for assignment in sets.iter() {
            let name = match assignment {
                ConflictAssignment::Column(name) | ConflictAssignment::Expr(name, _) => {
                    name.to_string()
                }
                _ => continue,
            };
            if self.columns.iter().any(|column| column.column == name) {
                return Err(immutable(&self.table, &name));
            }
        }
        Ok(())
    }

    /// Refuse an update setting any of `fields` that is a scope column, as
    /// Rust's `Error::ImmutableColumn` refuses it: a row cannot leave its
    /// scope.
    pub fn immutable<'f>(&self, fields: impl IntoIterator<Item = &'f str>) -> Result<(), Failure> {
        for field in fields {
            if let Some(column) = self.columns.iter().find(|column| column.field == field) {
                return Err(immutable(&self.table, &column.column));
            }
        }
        Ok(())
    }
}

/// Rust's `Error::ImmutableColumn` for `table`'s `column`.
pub fn immutable(table: &str, column: &str) -> Failure {
    Failure::Construction(format!(
        "`{table}`.`{column}` is written when its row is inserted and never changed"
    ))
}

/// Refuse a declared column holding a scope where it is nullable, an array
/// or generated: a row is in one value of its scope, which the insert under
/// its context writes.
// [spec:pgorm:req:bindings.scopes]
pub fn check_column(nullable: bool, array: bool, generated: bool) -> Result<(), Failure> {
    let refusal = if nullable {
        "a scope column is never NULL: a row is in one value of its scope"
    } else if array {
        "a scope column holds one value of its scope, not an array"
    } else if generated {
        "a scope column is written by the insert under its context, never generated"
    } else {
        return Ok(());
    };
    Err(Failure::Construction(refusal.to_owned()))
}

/// A Rust scope type an application registers, so a program's value of its
/// kind becomes one: its name, as pgorm names a scope, and the kind its
/// values are given as, its `ValueType`'s column type.
// [spec:pgorm:req:bindings.scopes-registered]
pub struct RegisteredScope {
    /// The scope's name: its type's last path segment, as pgorm names it.
    pub name: &'static str,
    /// The scope's values as a column of its type takes them, by which a
    /// binding converts a program's value.
    pub column: ColumnInfo,
    id: TypeId,
    make: fn(Value) -> Option<Box<dyn Any + Send + Sync>>,
}

impl fmt::Debug for RegisteredScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredScope")
            .field("name", &self.name)
            .field("column", &self.column)
            .finish_non_exhaustive()
    }
}

/// `S`'s name as pgorm names a scope: the last segment of its path.
pub fn scope_name<S>() -> &'static str {
    let name = std::any::type_name::<S>();
    let bare = name.split('<').next().unwrap_or(name);
    bare.rsplit("::").next().unwrap_or(bare)
}

impl RegisteredScope {
    /// The scope `S`, whose values are its `ValueType`'s.
    pub fn of<S>() -> Self
    where
        S: Scope + ValueType + Send + Sync,
    {
        let name = scope_name::<S>();
        let ty = S::column_type();
        Self {
            name,
            column: ColumnInfo {
                name: name.to_owned(),
                json_key: name.to_owned(),
                sql_type: format!("{ty:?}"),
                rust_type: Some(std::any::type_name::<S>()),
                nullable: false,
                primary_key: false,
                tag: Tag::of_column(&ty),
                labels: None,
            },
            id: TypeId::of::<S>(),
            make: |value| {
                S::from_value(value)
                    .ok()
                    .map(|scope| Box::new(scope) as Box<dyn Any + Send + Sync>)
            },
        }
    }
}

/// The scopes a registry knows, shared with each of its registrations, so
/// a statement reads a scope registered after the registration it starts
/// from, as a join into another entity may.
// [spec:pgorm:req:bindings.scopes-registered]
#[derive(Clone, Debug, Default)]
pub struct ScopeList(Arc<RwLock<Vec<Arc<RegisteredScope>>>>);

impl ScopeList {
    /// Add `scope`, refused where a scope of its name is known: a context
    /// names its scopes by name alone.
    pub(crate) fn add(&self, scope: RegisteredScope) -> Result<(), Failure> {
        let mut scopes = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if scopes.iter().any(|known| known.name == scope.name) {
            return Err(Failure::Construction(format!(
                "a scope named `{}` is already registered",
                scope.name
            )));
        }
        scopes.push(Arc::new(scope));
        Ok(())
    }

    /// Every scope known, in the order they were registered.
    pub fn get(&self) -> Vec<Arc<RegisteredScope>> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A context of Rust scope values, made from a program's [`Context`] through
/// the scopes a registration knows: a `ScopeContext` written by hand,
/// providing every scope to the type checker and answering at run time with
/// what the program gave, so a registered entity's `_for` forms run under
/// it as they run under a derived context.
// [spec:pgorm:req:bindings.scopes-registered]
#[derive(Default)]
pub struct Scoping {
    held: Vec<(TypeId, Option<Box<dyn Any + Send + Sync>>)>,
}

impl fmt::Debug for Scoping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scoping")
            .field("held", &self.held.len())
            .finish()
    }
}

impl Scoping {
    /// `cx`'s values as the Rust scopes of `scopes` they name: a scope
    /// `scopes` does not list is no Rust scope a registered statement
    /// reads, and is left out; a value its scope's type does not take is
    /// refused.
    pub fn new(scopes: &[Arc<RegisteredScope>], cx: &Context<Value>) -> Result<Self, Failure> {
        let mut held = Vec::new();
        for (name, holding) in cx.held() {
            let Some(scope) = scopes.iter().find(|scope| scope.name == name) else {
                continue;
            };
            let value =
                match holding {
                    Holding::One(value) => Some((scope.make)(value.clone()).ok_or_else(|| {
                        Failure::Construction(format!("{value:?} is no `{name}`"))
                    })?),
                    Holding::Every => None,
                };
            held.push((scope.id, value));
        }
        Ok(Self { held })
    }
}

impl ScopeContext for Scoping {
    fn held(&self, scope: TypeId) -> Option<Held<'_, dyn Any>> {
        self.held
            .iter()
            .find(|(id, _)| *id == scope)
            .map(|(_, value)| match value {
                Some(value) => Held::One(value.as_ref() as &dyn Any),
                None => Held::Every,
            })
    }
}

impl<S: Scope> Provides<S> for Scoping {}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> Result<DeclaredScopes, Failure> {
        DeclaredScopes::new(
            "note",
            vec![
                ScopeColumn {
                    field: "company".to_owned(),
                    column: "company_id".to_owned(),
                    scope: "CompanyId".to_owned(),
                },
                ScopeColumn {
                    field: "author".to_owned(),
                    column: "author_id".to_owned(),
                    scope: "UserId".to_owned(),
                },
            ],
        )
    }

    fn member() -> Result<Context<i64>, Failure> {
        Context::new(
            "Member",
            vec![
                ("CompanyId".to_owned(), Holding::One(1)),
                ("UserId".to_owned(), Holding::One(10)),
            ],
        )
    }

    fn admin() -> Result<Context<i64>, Failure> {
        Context::new(
            "CompanyAdmin",
            vec![
                ("CompanyId".to_owned(), Holding::One(1)),
                ("UserId".to_owned(), Holding::Every),
            ],
        )
    }

    // [spec:pgorm:req:bindings.scopes/test]    a read is held to each scope its context holds one value of, and is refused under no context or one missing a scope
    #[test]
    fn a_read_is_held_to_each_held_value() -> Result<(), Failure> {
        let scopes = note()?;
        let member = member()?;
        assert_eq!(
            scopes.condition(Some(&member), Dialect::Python).ok(),
            Some(vec![("company", &1), ("author", &10)])
        );
        assert_eq!(
            scopes.condition(Some(&admin()?), Dialect::Python).ok(),
            Some(vec![("company", &1)])
        );
        assert_eq!(
            scopes.condition::<i64>(None, Dialect::JavaScript).err(),
            Some(Failure::Construction(
                "`note` is scoped by `CompanyId` and `UserId`, which a statement taking no \
                 context does not hold: a statement on `note` starts from a `For` form, given \
                 a context holding its scopes first: `findFor(cx)`, `insertFor(cx, values)`, \
                 `updateFor(cx, values)`"
                    .to_owned()
            ))
        );
        let company = Context::new("CompanyId", vec![("CompanyId".to_owned(), Holding::One(1))])?;
        assert_eq!(
            scopes.condition(Some(&company), Dialect::Python).err(),
            Some(Failure::Construction(
                "`note` is scoped by `CompanyId` and `UserId`, which `CompanyId` does not hold"
                    .to_owned()
            ))
        );
        assert!(check("note", &["CompanyId"], Some(&company), Dialect::Python).is_ok());
        assert!(check("note", &["UserId"], Some(&company), Dialect::Python).is_err());
        assert!(check::<i64>("cake", &[], None, Dialect::Python).is_ok());
        Ok(())
    }

    // [spec:pgorm:req:bindings.scopes/test]    an insert fills what its row leaves unset, keeps the context's value, and refuses another value or an unset column under every value
    #[test]
    fn an_insert_fills_or_refuses_its_row() -> Result<(), Failure> {
        let scopes = note()?;
        let (member, admin) = (member()?, admin()?);
        let convert = |_: &ScopeColumn, value: &i64| Ok::<_, ()>(*value);
        let filled = scopes.fill(Some(&member), |_| None, convert, Dialect::Python);
        assert_eq!(filled, Ok(Ok(vec![("company", 1), ("author", 10)])));
        let kept = scopes.fill(
            Some(&member),
            |field| (field == "author").then_some(10),
            convert,
            Dialect::Python,
        );
        assert_eq!(kept, Ok(Ok(vec![("company", 1)])));
        let elsewhere = scopes.fill(
            Some(&member),
            |field| (field == "company").then_some(2),
            convert,
            Dialect::Python,
        );
        assert_eq!(
            elsewhere,
            Ok(Err(Failure::Construction(
                "the row sets `note`.`company_id` to a value outside the context's `CompanyId`"
                    .to_owned()
            )))
        );
        let unset = scopes.fill(Some(&admin), |_| None, convert, Dialect::Python);
        assert_eq!(
            unset,
            Ok(Err(Failure::Construction(
                "the context holds every `UserId`, so the row sets `note`.`author_id` itself, \
                 and it is not set"
                    .to_owned()
            )))
        );
        let authored = scopes.fill(
            Some(&admin),
            |field| (field == "author").then_some(11),
            convert,
            Dialect::Python,
        );
        assert_eq!(authored, Ok(Ok(vec![("company", 1)])));
        Ok(())
    }

    // [spec:pgorm:req:bindings.scopes/test]    a scope column is never updated, a context holds each scope once, and a model holds each scope in one column
    #[test]
    fn scope_columns_are_immutable_and_held_once() -> Result<(), Failure> {
        let scopes = note()?;
        assert_eq!(
            scopes.immutable(["body", "author"]),
            Err(Failure::Construction(
                "`note`.`author_id` is written when its row is inserted and never changed"
                    .to_owned()
            ))
        );
        assert!(scopes.immutable(["body"]).is_ok());
        assert!(
            Context::new(
                "Twice",
                vec![
                    ("UserId".to_owned(), Holding::One(1)),
                    ("UserId".to_owned(), Holding::<i64>::Every),
                ],
            )
            .is_err()
        );
        let twice = DeclaredScopes::new(
            "note",
            vec![
                ScopeColumn {
                    field: "author".to_owned(),
                    column: "author_id".to_owned(),
                    scope: "UserId".to_owned(),
                },
                ScopeColumn {
                    field: "editor".to_owned(),
                    column: "editor_id".to_owned(),
                    scope: "UserId".to_owned(),
                },
            ],
        );
        assert!(twice.is_err());
        use pgorm::pgorm_query::{Name, OnConflict};
        let upsert: OnConflict = OnConflict::column(Name::runtime("id"))
            .update_column(Name::runtime("author_id"))
            .into();
        assert_eq!(
            scopes.upsert(&upsert),
            Err(Failure::Construction(
                "`note`.`author_id` is written when its row is inserted and never changed"
                    .to_owned()
            ))
        );
        let body: OnConflict = OnConflict::column(Name::runtime("id"))
            .update_column(Name::runtime("body"))
            .into();
        assert!(scopes.upsert(&body).is_ok());
        assert_eq!(
            unscoped_join("note", &["CompanyId"], Dialect::JavaScript),
            Failure::Construction(
                "`note` is scoped by `CompanyId`; join it with a form taking the statement's \
                 context, such as `joinFor`"
                    .to_owned()
            )
        );
        Ok(())
    }
}
