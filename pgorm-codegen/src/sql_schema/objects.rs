use super::{Declared, TypeIdentity, table, types, unresolved, unsupported};
use crate::{Error, TableIdent, util::repeated_column};
use pg_query::NodeEnum;
use pg_query::protobuf::{
    CommentStmt, ConstrType, Constraint, CreateDomainStmt, CreateEnumStmt, CreateRangeStmt,
    CreateSchemaStmt, IndexStmt, ObjectType, RoleSpecType, SortByDir, SortByNulls,
};
use pgorm_query::{ColumnType, Name, TableKey, Unique};

/// The full identity — schema and name — a `CREATE TYPE` declares.
// [spec:pgorm:sem:codegen.ddl.objects+10]
fn type_identity(type_name: &[pg_query::protobuf::Node], at: usize) -> Result<TypeIdentity, Error> {
    declared_identity(type_name, "CREATE TYPE", at)
}

/// The full identity a statement creating a type declares, `statement` naming
/// it in an error.
// [spec:pgorm:sem:codegen.ddl.objects+10]
fn declared_identity(
    type_name: &[pg_query::protobuf::Node],
    statement: &str,
    at: usize,
) -> Result<TypeIdentity, Error> {
    let names = types::idents(type_name)
        .ok_or_else(|| unsupported(format!("a computed type name in {statement}"), at))?;
    match names.as_slice() {
        [name] => Ok((None, name.clone())),
        [schema, name] => Ok((Some(schema.clone()), name.clone())),
        [] => Err(unresolved(format!("{statement} without a type name"), at)),
        _ => Err(unsupported(
            format!("a catalog-qualified type name in {statement}"),
            at,
        )),
    }
}

/// A `CREATE DOMAIN` as its full identity and the column type of its base,
/// read as a column's type is and so against the types declared before it,
/// as PostgreSQL requires the base to exist — an enum, a range type or
/// another domain among them.
///
/// The base is all a column of the domain carries into `ColumnType::Domain`.
/// The domain's `COLLATE`, `DEFAULT`, `NOT NULL` and `CHECK`s are read and
/// set aside: the server applies them to every column of the domain, and none
/// changes what a value of it is. A constraint PostgreSQL refuses on a domain
/// is named, as the server would refuse the file.
// [spec:pgorm:sem:codegen.ddl.objects+10]
// [spec:pgorm:req:codegen.ddl.unsupported+18]
pub(super) fn domain(
    stmt: &CreateDomainStmt,
    declared: &Declared,
    at: usize,
) -> Result<(TypeIdentity, ColumnType), Error> {
    let identity = declared_identity(&stmt.domainname, "CREATE DOMAIN", at)?;
    let spelled = types::spell_identity(&identity);
    let Some(type_name) = stmt.type_name.as_ref() else {
        return Err(unresolved(
            format!("domain `{spelled}` has no base type"),
            at,
        ));
    };
    let context = format!("the base type of domain `{spelled}`");
    let base = types::column_kind(type_name, declared, &context, at)?;
    if base.auto_increment {
        return Err(unsupported(format!("a serial type as {context}"), at));
    }
    for node in &stmt.constraints {
        let Some(NodeEnum::Constraint(constraint)) = &node.node else {
            return Err(unsupported(
                format!("an unrecognised constraint on domain `{spelled}`"),
                at,
            ));
        };
        if let Some(what) = refused_on_domain(constraint) {
            return Err(unsupported(format!("{what} on domain `{spelled}`"), at));
        }
    }
    Ok((identity, base.col_type))
}

/// How a constraint PostgreSQL refuses on a domain was written, or `None` for
/// the four it takes.
// [spec:pgorm:req:codegen.ddl.unsupported+18]
fn refused_on_domain(constraint: &Constraint) -> Option<&'static str> {
    let kind = ConstrType::try_from(constraint.contype).ok();
    if constraint.is_no_inherit {
        return Some("a NO INHERIT constraint");
    }
    match kind {
        Some(
            ConstrType::ConstrNull
            | ConstrType::ConstrNotnull
            | ConstrType::ConstrDefault
            | ConstrType::ConstrCheck,
        ) => None,
        Some(ConstrType::ConstrPrimary) => Some("a PRIMARY KEY"),
        Some(ConstrType::ConstrUnique) => Some("a UNIQUE constraint"),
        Some(ConstrType::ConstrForeign) => Some("a REFERENCES clause"),
        Some(ConstrType::ConstrGenerated) => Some("a GENERATED clause"),
        Some(ConstrType::ConstrIdentity) => Some("an identity clause"),
        Some(
            ConstrType::ConstrAttrDeferrable
            | ConstrType::ConstrAttrNotDeferrable
            | ConstrType::ConstrAttrDeferred
            | ConstrType::ConstrAttrImmediate,
        ) => Some("a deferrable constraint"),
        Some(ConstrType::ConstrAttrNotEnforced) => Some("a NOT ENFORCED constraint"),
        Some(ConstrType::ConstrAttrEnforced) => Some("an ENFORCED clause"),
        _ => Some("an unrecognised constraint"),
    }
}

/// The schema a `CREATE SCHEMA` creates, by name, or `None` when the name is
/// the role a session runs as (`CREATE SCHEMA AUTHORIZATION CURRENT_USER`),
/// which the file does not say.
///
/// The schema holds no fact the entity model does: a table or type in it
/// carries its qualifier itself. Its `AUTHORIZATION` role is read and set
/// aside: who owns the schema is a privilege, not a fact about any value an
/// entity reads or writes. A schema element — a `CREATE TABLE` or other statement written
/// inside the `CREATE SCHEMA` — is named: the server qualifies an element's
/// unqualified names with the schema when it runs it, which the parse tree
/// does not, so reading one would place its table outside the schema.
// [spec:pgorm:sem:codegen.ddl.objects+10]
// [spec:pgorm:req:codegen.ddl.unsupported+18]
pub(super) fn schema(stmt: &CreateSchemaStmt, at: usize) -> Result<Option<String>, Error> {
    let name = match (stmt.schemaname.as_str(), &stmt.authrole) {
        ("", Some(role)) if role.roletype == RoleSpecType::RolespecCstring as i32 => {
            Some(role.rolename.clone())
        }
        ("", _) => None,
        (name, _) => Some(name.to_owned()),
    };
    if let Some(element) = stmt.schema_elts.first() {
        let kind = match &element.node {
            Some(NodeEnum::CreateStmt(_)) => "CREATE TABLE",
            Some(NodeEnum::IndexStmt(_)) => "CREATE INDEX",
            Some(other) => super::statement_kind(other),
            None => "an empty statement",
        };
        let schema = match &name {
            Some(name) => format!(" `{name}`"),
            None => String::new(),
        };
        return Err(unsupported(
            format!("{kind} inside CREATE SCHEMA{schema}"),
            at,
        ));
    }
    Ok(name)
}

/// A `CREATE TYPE ... AS ENUM` as the full identity — schema and name — and
/// values a column of that type carries into `ColumnType::Enum`.
// [spec:pgorm:sem:codegen.ddl.objects+10]
pub(super) fn enum_type(
    stmt: &CreateEnumStmt,
    at: usize,
) -> Result<(TypeIdentity, Vec<String>), Error> {
    let identity = type_identity(&stmt.type_name, at)?;
    let values = types::idents(&stmt.vals)
        .ok_or_else(|| unsupported("a computed value in CREATE TYPE", at))?;
    Ok((identity, values))
}

/// What a `CREATE TYPE ... AS RANGE` declares: the range type, the multirange
/// PostgreSQL creates beside it, and the column type of the subtype both range
/// over.
pub(super) struct ParsedRange {
    pub(super) range: TypeIdentity,
    pub(super) multirange: TypeIdentity,
    pub(super) subtype: ColumnType,
}

/// A `CREATE TYPE ... AS RANGE` as the full identities of its range and its
/// multirange and the column type of its subtype, read as a column's type is
/// and so against the types declared before it, as PostgreSQL requires the
/// subtype to exist.
///
/// The subtype is all a column of either carries into
/// `ColumnType::CreatedRange` / `CreatedMultirange`: the operator class,
/// collation, canonical and difference functions say how the server orders,
/// normalises and measures the range, none of which changes the value a row
/// holds. The multirange is `MULTIRANGE_TYPE_NAME` as written — unqualified,
/// it is created where an unqualified name is, not beside the range — or
/// otherwise the name PostgreSQL derives, in the range's schema.
// [spec:pgorm:sem:codegen.ddl.objects+10]
// [spec:pgorm:req:codegen.ddl.unsupported+18]
pub(super) fn range_type(
    stmt: &CreateRangeStmt,
    declared: &Declared,
    at: usize,
) -> Result<ParsedRange, Error> {
    let range = type_identity(&stmt.type_name, at)?;
    let spelled = types::spell_identity(&range);
    let mut subtype = None;
    let mut multirange = None;
    for node in &stmt.params {
        let Some(NodeEnum::DefElem(option)) = &node.node else {
            return Err(unsupported(
                format!("an option of range type `{spelled}`"),
                at,
            ));
        };
        let arg = option.arg.as_ref().and_then(|arg| arg.node.as_ref());
        match option.defname.as_str() {
            "subtype" => {
                let Some(NodeEnum::TypeName(type_name)) = arg else {
                    return Err(unsupported(
                        format!("a SUBTYPE that is not a type name on range type `{spelled}`"),
                        at,
                    ));
                };
                let context = format!("the SUBTYPE of range type `{spelled}`");
                // `pg_range` records the subtype's type and no modifier, so a
                // range over `numeric(10, 2)` holds any `numeric`. The grammar
                // writes `char`'s implicit length of 1 as a modifier too.
                if !type_name.typmods.is_empty() {
                    return Err(unsupported(
                        format!("a type modifier PostgreSQL drops on {context}"),
                        at,
                    ));
                }
                let kind = types::column_kind(type_name, declared, &context, at)?;
                if kind.auto_increment {
                    return Err(unsupported(format!("a serial type as {context}"), at));
                }
                subtype = Some(kind.col_type);
            }
            "multirange_type_name" => {
                let Some(NodeEnum::TypeName(type_name)) = arg else {
                    return Err(unsupported(
                        format!(
                            "a MULTIRANGE_TYPE_NAME that is not a type name on range type `{spelled}`"
                        ),
                        at,
                    ));
                };
                multirange = Some(type_identity(&type_name.names, at)?);
            }
            "subtype_opclass" | "collation" | "canonical" | "subtype_diff" => {}
            other => {
                return Err(unsupported(
                    format!("option `{other}` on range type `{spelled}`"),
                    at,
                ));
            }
        }
    }
    let subtype =
        subtype.ok_or_else(|| unresolved(format!("range type `{spelled}` has no SUBTYPE"), at))?;
    let multirange =
        multirange.unwrap_or_else(|| (range.0.clone(), derived_multirange_name(&range.1)));
    Ok(ParsedRange {
        range,
        multirange,
        subtype,
    })
}

/// The multirange name PostgreSQL derives from a range type's: the first
/// `range` in it (case-sensitive) becomes `multirange`, or, when there is
/// none, `_multirange` follows the name cut to 52 bytes; the result is cut to
/// the 63 bytes of an identifier. Each cut falls on a character boundary.
// [spec:pgorm:sem:codegen.ddl.objects+10]
fn derived_multirange_name(range: &str) -> String {
    let derived = match range.find("range") {
        Some(at) => {
            let (head, tail) = range.split_at(at);
            format!("{head}multi{tail}")
        }
        None => format!("{}_multirange", clip(range, 52)),
    };
    clip(&derived, 63).to_owned()
}

/// The longest prefix of `text` within `bytes` bytes that ends on a character
/// boundary.
fn clip(text: &str, bytes: usize) -> &str {
    (0..=bytes.min(text.len()))
        .rev()
        .find_map(|end| text.get(..end))
        .unwrap_or_default()
}

/// A `CREATE INDEX`, and the table it belongs to.
pub(super) struct ParsedIndex {
    pub(super) table: TableIdent,
    /// A unique index as the unique key it enforces — `None` for an index
    /// that states no entity fact, which has no place inside a
    /// `CREATE TABLE`.
    pub(super) constraint: Option<TableKey<Unique>>,
}

// [spec:pgorm:sem:codegen.ddl.objects+10]
// [spec:pgorm:req:codegen.ddl.unsupported+18]
pub(super) fn index(stmt: &IndexStmt, at: usize) -> Result<ParsedIndex, Error> {
    let table = match stmt.relation.as_ref() {
        Some(relation) if !relation.relname.is_empty() => TableIdent {
            table: relation.relname.clone(),
            schema: table::schema_of(relation),
        },
        _ => return Err(unresolved("CREATE INDEX without a table name", at)),
    };
    let name = if stmt.idxname.is_empty() {
        format!("index on `{table}`")
    } else {
        format!("index `{}`", stmt.idxname)
    };
    let on = |what: &str| unsupported(format!("{what} on {name}"), at);
    if stmt.concurrent {
        return Err(on("CONCURRENTLY"));
    }
    if stmt.where_clause.is_some() {
        return Err(on("a WHERE clause"));
    }
    if !stmt.index_including_params.is_empty() {
        return Err(on("an INCLUDE clause"));
    }
    if !stmt.options.is_empty() {
        return Err(on("a WITH storage option"));
    }
    if !stmt.table_space.is_empty() {
        return Err(on("a TABLESPACE clause"));
    }
    if !stmt.exclude_op_names.is_empty() {
        return Err(on("an exclusion operator"));
    }

    let mut columns = Vec::with_capacity(stmt.index_params.len());
    for node in &stmt.index_params {
        let Some(NodeEnum::IndexElem(element)) = &node.node else {
            return Err(on("an index element"));
        };
        if element.name.is_empty() || element.expr.is_some() {
            return Err(on("an expression column"));
        }
        if !element.collation.is_empty() {
            return Err(on("a COLLATE clause"));
        }
        if !element.opclass.is_empty() || !element.opclassopts.is_empty() {
            return Err(on("an operator class"));
        }
        if element.nulls_ordering != SortByNulls::SortbyNullsDefault as i32 {
            return Err(on("a NULLS FIRST or NULLS LAST clause"));
        }
        let descending = match SortByDir::try_from(element.ordering) {
            Ok(SortByDir::SortbyDefault | SortByDir::SortbyAsc) => false,
            Ok(SortByDir::SortbyDesc) => true,
            _ => return Err(on("an index column ordering")),
        };
        columns.push((Name::runtime(element.name.as_str()), descending));
    }
    if columns.is_empty() {
        return Err(on("an index over no columns"));
    }
    if !stmt.unique {
        return Ok(ParsedIndex {
            table,
            constraint: None,
        });
    }

    // A unique index is carried as the table constraint that enforces the
    // same uniqueness, and a table constraint's key is plain column names
    // under the default btree: a descending key column or another access
    // method has no spelling there, so each is named rather than dropped.
    let on_unique = |what: &str| unsupported(format!("{what} on unique {name}"), at);
    if !matches!(stmt.access_method.as_str(), "" | "btree") {
        return Err(on_unique("an access method other than btree"));
    }
    if columns.iter().any(|(_, descending)| *descending) {
        return Err(on_unique("a DESC column"));
    }
    // The server takes a unique index over a column named twice, but not
    // the table constraint it folds into (42701).
    if let Some(column) = repeated_column(columns.iter().map(|(column, _)| column.to_string())) {
        return Err(on_unique(&format!("column `{column}` named twice")));
    }
    let mut columns = columns.into_iter().map(|(column, _)| column);
    let Some(first) = columns.next() else {
        return Err(on("an index over no columns"));
    };
    let mut constraint = TableKey::new(first).cols(columns);
    if stmt.nulls_not_distinct {
        constraint = constraint.nulls_not_distinct();
    }
    if !stmt.idxname.is_empty() {
        constraint = constraint.name(Name::runtime(stmt.idxname.as_str()));
    }
    Ok(ParsedIndex {
        table,
        constraint: Some(constraint),
    })
}

/// A `COMMENT ON` the bridge can attach to a table or one of its columns.
pub(super) enum ParsedComment {
    Table {
        table: TableIdent,
        text: String,
    },
    Column {
        table: TableIdent,
        column: String,
        text: String,
    },
}

impl ParsedComment {
    pub(super) fn table(&self) -> &TableIdent {
        match self {
            Self::Table { table, .. } | Self::Column { table, .. } => table,
        }
    }
}

// [spec:pgorm:sem:codegen.ddl.objects+10]
pub(super) fn comment(stmt: &CommentStmt, at: usize) -> Result<ParsedComment, Error> {
    let kind = match ObjectType::try_from(stmt.objtype) {
        Ok(kind @ (ObjectType::ObjectTable | ObjectType::ObjectColumn)) => kind,
        _ => {
            return Err(unsupported(
                "COMMENT ON an object other than a table or column",
                at,
            ));
        }
    };
    let names = match stmt.object.as_ref().and_then(|node| node.node.as_ref()) {
        Some(NodeEnum::List(list)) => types::idents(&list.items),
        _ => None,
    }
    .ok_or_else(|| unsupported("COMMENT ON a computed object name", at))?;
    let text = stmt.comment.clone();
    let named = |schema: Option<&String>, table: &String| TableIdent {
        table: table.clone(),
        schema: schema.cloned(),
    };
    match (kind, names.as_slice()) {
        (ObjectType::ObjectTable, [table]) => Ok(ParsedComment::Table {
            table: named(None, table),
            text,
        }),
        (ObjectType::ObjectTable, [schema, table]) => Ok(ParsedComment::Table {
            table: named(Some(schema), table),
            text,
        }),
        (ObjectType::ObjectColumn, [table, column]) => Ok(ParsedComment::Column {
            table: named(None, table),
            column: column.clone(),
            text,
        }),
        (ObjectType::ObjectColumn, [schema, table, column]) => Ok(ParsedComment::Column {
            table: named(Some(schema), table),
            column: column.clone(),
            text,
        }),
        _ => Err(unresolved("COMMENT ON names no readable object", at)),
    }
}
