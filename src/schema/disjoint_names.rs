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

impl DisjointShape {
    /// Refuse a unique key a described table declares
    /// ([`DisjointTable::with_unique_key`]) named as a constraint or index
    /// the disjoint schema adds — the base's `UNIQUE` key, a composite
    /// primary key's `pk-{table}`, the base's foreign key to a kind's table
    /// or a kind table's to its base row, on the key's own table — or as
    /// another table's unique key in its schema, each of which PostgreSQL
    /// refuses once the statements run, as [`Error::Type`] naming both. An
    /// unkinded shape writes no schema, and refuses nothing here.
    // [spec:pgorm:req:bindings.model-unique-keys+1]
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
        let mut taken = vec![Taken {
            schema: schema_of(&self.base),
            table: self.base.label.clone(),
            indexed: true,
            name: base.unique_name(),
            what: format!(
                "the `UNIQUE` key the disjoint schema adds to `{}`",
                self.base.label
            ),
        }];
        for table in &tables {
            if let Some(name) = primary_key_name(table) {
                taken.push(Taken {
                    schema: schema_of(table),
                    table: table.label.clone(),
                    indexed: true,
                    name,
                    what: format!("the primary key of `{}`", table.label),
                });
            }
        }
        for kind in &self.kinds {
            let Some(table) = &kind.table else { continue };
            taken.push(Taken {
                schema: schema_of(&self.base),
                table: self.base.label.clone(),
                indexed: false,
                name: base.generated(&kind.value).name,
                what: format!(
                    "the foreign key the disjoint schema adds to `{}` for the kind `{}`",
                    self.base.label, kind.value
                ),
            });
            taken.push(Taken {
                schema: schema_of(table),
                table: table.label.clone(),
                indexed: false,
                name: kind_key_name(&base, table),
                what: format!(
                    "the foreign key the disjoint schema adds to `{}` onto its base row",
                    table.label
                ),
            });
        }
        for table in &tables {
            for key in &table.unique_keys {
                let Some(name) = &key.name else { continue };
                if let Some(other) = taken.iter().find(|taken| taken.clashes(table, name)) {
                    return Err(Error::Type(format!(
                        "the unique key `{name}` of `{}` is named as {}: PostgreSQL holds one \
                         index of a name to a schema and one constraint of a name to a table",
                        table.label, other.what
                    )));
                }
                taken.push(Taken {
                    schema: schema_of(table),
                    table: table.label.clone(),
                    indexed: true,
                    name: name.clone(),
                    what: format!("a unique key of `{}`", table.label),
                });
            }
        }
        Ok(())
    }
}
