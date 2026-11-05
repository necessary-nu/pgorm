//! The names a described table's unique keys may not take: those of the
//! constraints and indexes the disjoint schema adds, and each other's.

use super::disjoint::{BaseTable, kind_key_name};
use crate::{DisjointShape, DisjointTable, Error, executor::result_name::derived_name};

/// A name taken on the shape's tables: by an index, which PostgreSQL holds
/// one of a name to a schema (`42P07`), or by a constraint, one of a name to
/// a table (`42710`); and what takes it.
struct Taken {
    schema: Option<String>,
    table: String,
    indexed: bool,
    name: String,
    what: String,
}

impl Taken {
    /// Whether a unique key named `name` on `table`, an index of the
    /// schema's, clashes with this name.
    fn clashes(&self, table: &DisjointTable, name: &str) -> bool {
        let schema = table.table.schema().map(|schema| schema.to_string());
        self.name == name
            && match self.indexed {
                true => self.schema == schema,
                false => self.table == table.label,
            }
    }
}

/// The composite primary key's name `table` is written with, `pk-{table}`,
/// or `None` for a key of one column, which PostgreSQL names.
pub(crate) fn primary_key_name(table: &DisjointTable) -> Option<String> {
    (table.key.len() > 1).then(|| derived_name("pk", &table.name().to_string(), []))
}

/// PostgreSQL's byte limit on an identifier, `NAMEDATALEN - 1`.
const IDENTIFIER_BYTES: usize = 63;

/// The name PostgreSQL gives an object it names itself (`makeObjectName`):
/// `name1`, `_name2` where there is one, and `_label`, the longer of the two
/// names shortened first, a byte at a time, until the whole fits 63 bytes,
/// and each then cut back to a character boundary.
fn implicit_name(name1: &str, name2: Option<&str>, label: &str) -> String {
    let overhead = usize::from(name2.is_some()) + label.len() + 1;
    let available = IDENTIFIER_BYTES.saturating_sub(overhead);
    let (mut first, mut second) = (name1.len(), name2.map_or(0, str::len));
    while first + second > available {
        if first > second {
            first -= 1;
        } else {
            second -= 1;
        }
    }
    let clip = |name: &str, mut length: usize| -> String {
        while !name.is_char_boundary(length) {
            length -= 1;
        }
        name[..length].to_owned()
    };
    let mut name = clip(name1, first);
    if let Some(name2) = name2 {
        name.push('_');
        name.push_str(&clip(name2, second));
    }
    name.push('_');
    name.push_str(label);
    name
}

/// The columns of an unnamed unique key as PostgreSQL's name for it holds
/// them (`ChooseIndexNameAddition`): joined by `_`, the joining stopped once
/// it reaches 64 bytes.
fn columns_part<'c>(columns: impl IntoIterator<Item = &'c str>) -> String {
    let mut part = String::new();
    for column in columns {
        if !part.is_empty() {
            part.push('_');
        }
        part.push_str(column);
        if part.len() > IDENTIFIER_BYTES {
            break;
        }
    }
    part
}

/// The names PostgreSQL gives the constraints and indexes of `table` it
/// names itself: an unnamed one-column primary key's `{table}_pkey`, an
/// unnamed unique key's `{table}_{columns}_key`, and each NOT NULL column's
/// `{table}_{column}_not_null`, PostgreSQL 18 naming its not-null
/// constraints, the key's columns among them.
fn implicit_names(table: &DisjointTable) -> Vec<(bool, String, String)> {
    let bare = table.name().to_string();
    let label = &table.label;
    let mut names = Vec::new();
    if table.key.len() == 1 {
        names.push((
            true,
            implicit_name(&bare, None, "pkey"),
            format!("the primary key PostgreSQL names on `{label}`"),
        ));
    }
    let mut unnamed: Vec<Vec<String>> = table
        .columns
        .iter()
        .filter(|column| column.def.unique)
        .map(|column| vec![column.name.to_string()])
        .collect();
    unnamed.extend(
        table
            .declared_keys
            .iter()
            .filter(|key| key.name.is_none())
            .map(|key| {
                key.columns
                    .iter()
                    .map(|&column| column.to_owned())
                    .collect()
            }),
    );
    unnamed.extend(
        table
            .unique_keys
            .iter()
            .filter(|key| key.name.is_none())
            .map(|key| key.columns.clone()),
    );
    for columns in unnamed {
        let part = columns_part(columns.iter().map(String::as_str));
        names.push((
            true,
            implicit_name(&bare, Some(&part), "key"),
            format!(
                "the unique key PostgreSQL names on `{label}` over {}",
                columns.join(", ")
            ),
        ));
    }
    for (index, column) in table.columns.iter().enumerate() {
        if column.def.is_null() && !table.key.contains(&index) {
            continue;
        }
        let name = column.name.to_string();
        names.push((
            false,
            implicit_name(&bare, Some(&name), "not_null"),
            format!("the not-null constraint PostgreSQL names on `{label}`.`{name}`"),
        ));
    }
    names
}

impl DisjointShape {
    /// Refuse a named unique key of the shape's tables — one a described
    /// table declares ([`DisjointTable::with_unique_key`]) or a typed table's
    /// entity declares (`unique(..)`) — named as a constraint or index the
    /// disjoint schema adds — the base's `UNIQUE` key, a composite primary
    /// key's `pk-{table}`, the base's foreign key to a kind's table or a kind
    /// table's to its base row, on the key's own table — as one PostgreSQL
    /// names itself (`{table}_pkey`, `{table}_{columns}_key`,
    /// `{table}_{column}_not_null`), or as another named key, each of which
    /// PostgreSQL refuses once the statements run, as [`Error::Type`] naming
    /// both. An index's name clashes within its schema and a constraint's
    /// within its table; a relation of the schema outside the shape is the
    /// server's to refuse. An unkinded shape writes no schema, and refuses
    /// nothing here.
    // [spec:pgorm:req:bindings.model-unique-keys+2]
    pub(crate) fn keys_named_apart(&self) -> Result<(), Error> {
        let Some(kinded) = &self.kinded else {
            return Ok(());
        };
        let base = BaseTable::of(self, kinded);
        let tables: Vec<&DisjointTable> = std::iter::once(&self.base)
            .chain(self.kinds.iter().filter_map(|kind| kind.table.as_ref()))
            .collect();
        let schema_of =
            |table: &DisjointTable| table.table.schema().map(|schema| schema.to_string());
        let taken_on = |table: &DisjointTable, indexed: bool, name: String, what: String| Taken {
            schema: schema_of(table),
            table: table.label.clone(),
            indexed,
            name,
            what,
        };
        let mut taken = vec![taken_on(
            &self.base,
            true,
            base.unique_name(),
            format!(
                "the `UNIQUE` key the disjoint schema adds to `{}`",
                self.base.label
            ),
        )];
        for table in &tables {
            if let Some(name) = primary_key_name(table) {
                taken.push(taken_on(
                    table,
                    true,
                    name,
                    format!("the primary key of `{}`", table.label),
                ));
            }
            for (indexed, name, what) in implicit_names(table) {
                taken.push(taken_on(table, indexed, name, what));
            }
        }
        for kind in &self.kinds {
            let Some(table) = &kind.table else { continue };
            taken.push(taken_on(
                &self.base,
                false,
                base.generated(&kind.value).name,
                format!(
                    "the foreign key the disjoint schema adds to `{}` for the kind `{}`",
                    self.base.label, kind.value
                ),
            ));
            taken.push(taken_on(
                table,
                false,
                kind_key_name(&base, table),
                format!(
                    "the foreign key the disjoint schema adds to `{}` onto its base row",
                    table.label
                ),
            ));
        }
        for table in &tables {
            let named = table
                .declared_keys
                .iter()
                .filter_map(|key| key.name.map(str::to_owned))
                .chain(table.unique_keys.iter().filter_map(|key| key.name.clone()));
            for name in named {
                if let Some(other) = taken.iter().find(|taken| taken.clashes(table, &name)) {
                    return Err(Error::Type(format!(
                        "the unique key `{name}` of `{}` is named as {}: PostgreSQL holds one \
                         index of a name to a schema and one constraint of a name to a table",
                        table.label, other.what
                    )));
                }
                let what = format!("a unique key of `{}`", table.label);
                taken.push(taken_on(table, true, name, what));
            }
        }
        Ok(())
    }
}
