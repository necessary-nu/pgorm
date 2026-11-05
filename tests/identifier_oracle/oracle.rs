//! The structural engine: render a name at a site, parse the statement with
//! libpg_query, and compare its parse tree with the tree of the same statement
//! rendered with a benign name.
//!
//! The comparison is generic. Both trees are libpg_query's protobuf serialised
//! to JSON; they are walked in parallel, `location`-style fields skipped, and
//! must agree on every key, every array length and every leaf — except the
//! leaves where the benign tree holds [`BENIGN`], which are the name's
//! positions and must hold the hostile name instead (or, for a name the API
//! derives from the caller's — `pk-{table}` — the same derivation of it,
//! bounded to 63 bytes as pgorm bounds every name it composes). No
//! per-site matcher is written: a site declares only the node kinds its name
//! should land in, and the walk both finds those positions and proves nothing
//! else moved.

use std::fmt::Write as _;

use bytes::BytesMut;
use postgres_protocol::message::frontend;
use serde_json::Value as Json;

use super::corpus::{Hostile, NUL_NAME};

/// The name every site is also rendered with, to produce the reference tree.
///
/// A safe lowercase identifier, so it renders identically under every policy
/// (bare where a policy allows bare names, quoted elsewhere — the parser
/// reads both spellings as the same name).
pub const BENIGN: &str = "oracle_benign";

/// PostgreSQL's `NAMEDATALEN`: an identifier keeps at most one byte fewer.
const NAMEDATALEN: usize = 64;

/// The longest identifier PostgreSQL keeps whole, in bytes.
pub const IDENTIFIER_BYTES: usize = NAMEDATALEN - 1;

/// Fields that record where in the text a node was, not what it is. They
/// differ between two renderings whose names differ in length, and carry no
/// structure.
///
/// Every byte-offset field libpg_query's protobuf carries: the `location` of
/// most nodes and a statement's extent, and the offsets PostgreSQL 18 added
/// beside them — an `IN` list's bounds on `A_Expr`, an array constructor's
/// bounds on `A_ArrayExpr` and `ArrayExpr`, the value of a `DefElem`, a
/// `NOTIFY` payload and a subscription's connection string — plus a JSON
/// table path's name.
const POSITION_KEYS: [&str; 11] = [
    "location",
    "stmt_location",
    "stmt_len",
    "rexpr_list_start",
    "rexpr_list_end",
    "list_start",
    "list_end",
    "arg_location",
    "payload_location",
    "conninfo_location",
    "name_location",
];

/// Node kinds that only wrap a value; a name's position is named by the node
/// that holds the wrapper, not by the wrapper.
const VALUE_WRAPPERS: [&str; 3] = ["String", "Sval", "sval"];

/// Corpus names PRQL binds for itself: `pipeline::error::RESERVED` lists
/// them, so the alias screen refuses them, and prqlc's name resolution reads
/// them as its own `std` items wherever they stand unqualified. `user`,
/// `integer`, `left`, `row`, `distinct` and `only` are not among them.
const PRQL_RESERVED_IN_CORPUS: [&str; 4] = ["select", "from", "not", "coalesce"];

/// The keywords `sql.types.type-name` writes bare in an unqualified type
/// position because the grammar reads each as one of its own type spellings,
/// resolving to a catalogue type (`integer` → `int4`). Listed per keyword,
/// as the rule lists them: nothing else may read as a type keyword.
pub const TYPE_SPELLINGS: [&str; 19] = [
    "bigint",
    "bit",
    "boolean",
    "char",
    "character",
    "dec",
    "decimal",
    "float",
    "int",
    "integer",
    "interval",
    "json",
    "nchar",
    "numeric",
    "real",
    "smallint",
    "time",
    "timestamp",
    "varchar",
];

/// The keywords `sql.types.type-name` writes bare at a function position
/// because the grammar reads each call as an expression of its own rather
/// than a function in `pg_proc`, each with the node the parser builds in
/// place of the call. Listed per keyword, as the rule lists them.
pub const CALL_FORMS: [(&str, &str); 4] = [
    ("coalesce", "CoalesceExpr"),
    ("greatest", "MinMaxExpr"),
    ("least", "MinMaxExpr"),
    ("nullif", "AExpr"),
];

/// What a site did with a name.
#[derive(Debug)]
pub enum Rendered {
    /// The statement text the API produced.
    Sql(String),
    /// The API refused the name; the payload is the error's `Debug` spelling.
    Refused(String),
}

impl Rendered {
    pub fn sql(sql: impl Into<String>) -> Self {
        Self::Sql(sql.into())
    }

    pub fn from_result<E: std::fmt::Debug>(result: Result<String, E>) -> Self {
        match result {
            Ok(sql) => Self::Sql(sql),
            Err(err) => Self::Refused(format!("{err:?}")),
        }
    }
}

/// How a site writes a caller-supplied name, which fixes what the oracle
/// expects of each corpus entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// `SqlName::prepare`: always double-quoted, embedded `"` doubled.
    Quoted,
    /// `TypeName`'s part policy (`sql.types.type-name`) at a type position,
    /// or an index access method: bare when the name matches
    /// `^[a-z_][a-z0-9_]*$` and is no keyword PostgreSQL restricts, or is
    /// one of [`TYPE_SPELLINGS`] in an unqualified type; quoted like
    /// [`Quoted`](Self::Quoted) otherwise.
    TypePart,
    /// The same policy at a function position (`Func::named`), where the
    /// keywords written bare are [`CALL_FORMS`] instead, and the unreserved
    /// `operator` is quoted because `operator(` opens `OPERATOR(schema.op)`.
    FunctionName,
    /// A string literal rather than an identifier: enum labels, which are
    /// values. Inline rendering escapes them (`E'…'` when a backslash or a
    /// control character is present).
    Literal,
    /// An enum label pgorm also composes identifiers from, in one statement:
    /// a disjoint kind's value, which names the base's generated key columns
    /// and their foreign key, and the `CHECK` refusing a removed kind. Where
    /// it stands alone it is a [`Literal`](Self::Literal); within a name it
    /// is part of an identifier written as [`Quoted`](Self::Quoted) writes
    /// one, so a NUL reaches the text.
    DerivedLabel,
    /// A qualified pipeline identifier (a column under its relation, a schema
    /// or the table after it): refused with `UnquotableIdentifier` when it
    /// carries `"` or NUL, begins with `$`, or is `*`; otherwise written by
    /// prqlc — bare when its `valid_ident` accepts the name and it is not a
    /// keyword, quoted with `"` doubled when not.
    Pipeline,
    /// An unqualified pipeline identifier — a relation, or a column reference
    /// standing alone: refused like [`Pipeline`](Self::Pipeline), and also by
    /// prqlc with `Compile` when PRQL's `std` binds the name, since a PRQL
    /// built-in used as a value is a name-resolution failure
    /// (`PipelineError::Compile`).
    PipelineBare,
    /// A pipeline alias: refused like [`Pipeline`](Self::Pipeline), and also
    /// with `ReservedAlias` when PRQL reserves the name.
    PipelineAlias,
}

/// What a site does with a NUL-bearing name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NulBehaviour {
    /// The API refuses it before rendering.
    Refused,
    /// The API renders it; the NUL byte reaches the statement text, and the
    /// protocol encoder refuses the text before anything is sent.
    Encoder,
    /// The API renders it as an escape (`E'\0'`) with no NUL byte in the
    /// text; the grammar refuses the escape.
    Escaped,
}

impl Policy {
    /// The refusal this policy owes `name`, if any.
    fn refusal(self, name: &str) -> Option<&'static str> {
        match self {
            Self::Pipeline | Self::PipelineBare | Self::PipelineAlias
                if name.contains(['"', '\0']) || name.starts_with('$') || name == "*" =>
            {
                Some("UnquotableIdentifier")
            }
            Self::PipelineBare if PRQL_RESERVED_IN_CORPUS.contains(&name) => Some("Compile"),
            Self::PipelineAlias if PRQL_RESERVED_IN_CORPUS.contains(&name) => Some("ReservedAlias"),
            _ => None,
        }
    }

    pub fn nul(self) -> NulBehaviour {
        match self {
            Self::Quoted | Self::TypePart | Self::FunctionName | Self::DerivedLabel => {
                NulBehaviour::Encoder
            }
            Self::Literal => NulBehaviour::Escaped,
            Self::Pipeline | Self::PipelineBare | Self::PipelineAlias => NulBehaviour::Refused,
        }
    }

    /// The text the parser hands back for `name` at this site. Identifiers
    /// are truncated by the scanner itself to `NAMEDATALEN - 1` bytes, on a
    /// character boundary, exactly as the server does; a literal is not.
    fn parsed_form(self, name: &str) -> String {
        if self == Self::Literal {
            return name.to_owned();
        }
        truncate_identifier(name)
    }

    /// The text the parser hands back at a position whose benign text is
    /// `benign`, for the hostile `name`. The name itself reads back as
    /// [`parsed_form`](Self::parsed_form) says. A name pgorm derives from it
    /// — `idx-{table}-{column}`, `pk-{table}`, `fk-{table}-{columns}`, whose
    /// benign text holds [`BENIGN`] among other text — is the same
    /// derivation of the hostile name, bounded as pgorm bounds every name it
    /// composes rather than left for the server to truncate
    /// (`schema.from-entity`): the bounding is pgorm's own, through the
    /// public function prefixed result names take, so the oracle holds the
    /// position to exactly the name pgorm writes without composing it again.
    /// A [`DerivedLabel`](Self::DerivedLabel) standing alone is a literal,
    /// its bytes whatever their length.
    fn expected(self, benign: &str, name: &str) -> String {
        let substituted = benign.replace(BENIGN, name);
        if self == Self::DerivedLabel && benign == BENIGN {
            return substituted;
        }
        if benign == BENIGN || self == Self::Literal {
            return self.parsed_form(&substituted);
        }
        self.parsed_form(&pgorm::result_column_name("", &substituted))
    }

    /// Whether `hostile` is what a position whose benign text is `benign`
    /// reads back as for `name`: [`expected`](Self::expected), or, where a
    /// `-`-separated part of a derived name is itself a name pgorm composed
    /// and bounded — a disjoint base's `fk-{base}-{value}_{column}`, whose
    /// last part is its generated column's name — that derivation with the
    /// part bounded first, as pgorm bounds it.
    fn reads_back(self, benign: &str, name: &str, hostile: &str) -> bool {
        if hostile == self.expected(benign, name) {
            return true;
        }
        if benign == BENIGN || self == Self::Literal {
            return false;
        }
        let parts: Vec<&str> = benign.split('-').collect();
        (0..parts.len())
            .filter(|&index| parts[index].contains(BENIGN) && parts[index] != BENIGN)
            .any(|bounded| {
                let composed = parts
                    .iter()
                    .enumerate()
                    .map(|(index, part)| {
                        let part = part.replace(BENIGN, name);
                        match index == bounded {
                            true => pgorm::result_column_name("", &part),
                            false => part,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("-");
                hostile == self.parsed_form(&pgorm::result_column_name("", &composed))
            })
    }
}

/// `name` cut to at most `NAMEDATALEN - 1` bytes without splitting a
/// character — PostgreSQL's `truncate_identifier` under UTF-8.
pub fn truncate_identifier(name: &str) -> String {
    let mut end = name.len().min(IDENTIFIER_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

/// One registered identifier render site.
pub struct Site {
    /// `<crate>/<area>.<position>`, unique across the registry.
    pub id: &'static str,
    /// The public API a caller reaches this position through.
    pub api: &'static str,
    /// The parse-tree positions the name must land in, one per occurrence,
    /// in the order a walk of the tree meets them (`serde_json` keeps an
    /// object's fields sorted, so this is field-name order, not text order):
    /// the node kind that holds it, then the field path (see [`kind_of`]).
    pub kinds: &'static [&'static str],
    pub policy: Policy,
    /// Build the statement with `name` in this site's position, and return
    /// its text — or the API's refusal.
    pub render: fn(&str) -> Rendered,
}

impl Site {
    pub fn crate_name(&self) -> &'static str {
        self.id.split('/').next().unwrap_or(self.id)
    }
}

/// How a corpus name fared at a site, when it fared as the policy requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Verdict {
    /// Rendered, parsed as one statement, the name back intact at exactly the
    /// declared positions, and every other node identical to the benign tree.
    RoundTrip,
    /// Refused by the API, as the policy says it must be.
    Refused(&'static str),
    /// The empty name: rendered as `""`, which the grammar itself refuses as a
    /// zero-length delimited identifier, so no statement exists to run.
    EmptyRejected,
    /// One of [`TYPE_SPELLINGS`] at a [`TypePart`](Policy::TypePart) site,
    /// which the grammar reads as its own type spelling (`integer`),
    /// resolving to that type as `sql.types.type-name` intends: the parse
    /// differs from the benign one inside the type's own `TypeName` node and
    /// nowhere else.
    TypeKeyword,
    /// One of [`CALL_FORMS`] at a [`FunctionName`](Policy::FunctionName)
    /// site, which the grammar reads as its own expression
    /// (`coalesce(1)` is a `CoalesceExpr`) as `sql.types.type-name` intends:
    /// the call node became that form's node and nothing outside it differs
    /// from the benign tree.
    CallForm,
}

/// A step in a path through a parse tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// Parse `sql` and return its protobuf parse tree as JSON.
pub fn parse_tree(sql: &str) -> Result<Json, String> {
    let parsed =
        pg_query::parse(sql, pg_query::ParserOptions::DEFAULT).map_err(|err| err.to_string())?;
    serde_json::to_value(&parsed.protobuf).map_err(|err| err.to_string())
}

/// `sql` quoted for a failure message: `Debug` spelling, so a newline, a tab,
/// a NUL or a bidi override is visible rather than acted on.
fn show(sql: &str) -> String {
    format!("{sql:?}")
}

fn failure(site: &Site, hostile: &Hostile, what: String) -> String {
    format!(
        "site `{}` ({}), name `{}` {:?}: {what}",
        site.id, site.api, hostile.label, hostile.name
    )
}

/// Hold `site` to the property for one corpus name.
pub fn judge(site: &Site, hostile: &Hostile) -> Result<Verdict, String> {
    let reference = reference(site).map_err(|what| failure(site, hostile, what))?;
    judge_against(site, hostile, (site.render)(&hostile.name), &reference)
}

/// A site's benign rendering, parsed, with the positions its name occupies.
pub struct Reference {
    sql: String,
    tree: Json,
    positions: Vec<Vec<Seg>>,
}

/// Hold one rendering of `site` against its [`Reference`].
pub fn judge_against(
    site: &Site,
    hostile: &Hostile,
    rendered: Rendered,
    reference: &Reference,
) -> Result<Verdict, String> {
    let name = hostile.name.as_str();

    if let Some(expected) = site.policy.refusal(name) {
        return match rendered {
            Rendered::Refused(err) if err.contains(expected) => Ok(Verdict::Refused(expected)),
            Rendered::Refused(err) => Err(failure(
                site,
                hostile,
                format!("refused, but not with {expected}: {err}"),
            )),
            Rendered::Sql(sql) => Err(failure(
                site,
                hostile,
                format!(
                    "rendered a name its API must refuse ({expected}):\n    {}",
                    show(&sql)
                ),
            )),
        };
    }

    let sql = match rendered {
        Rendered::Sql(sql) => sql,
        Rendered::Refused(err) => {
            return Err(failure(
                site,
                hostile,
                format!("refused a name its policy renders: {err}"),
            ));
        }
    };
    let Reference {
        sql: benign_sql,
        tree: benign,
        positions,
    } = reference;

    let tree = match parse_tree(&sql) {
        Ok(tree) => tree,
        Err(err)
            if name.is_empty()
                && site.policy != Policy::Literal
                && err.contains("zero-length delimited identifier") =>
        {
            return Ok(Verdict::EmptyRejected);
        }
        Err(err) => {
            return Err(failure(
                site,
                hostile,
                format!(
                    "the rendered statement does not parse: {err}\n    {}",
                    show(&sql)
                ),
            ));
        }
    };

    let statements = tree["stmts"].as_array().map_or(0, Vec::len);
    if statements != 1 {
        return Err(failure(
            site,
            hostile,
            format!(
                "rendered {statements} statements, not one:\n    {}",
                show(&sql)
            ),
        ));
    }

    let mut walk = Walk {
        name,
        policy: site.policy,
        path: Vec::new(),
        skip: Vec::new(),
        diffs: Vec::new(),
    };
    walk.compare(benign, &tree);
    if !walk.diffs.is_empty() && type_keyword(site, name, positions, benign, &tree) {
        return Ok(Verdict::TypeKeyword);
    }
    if !walk.diffs.is_empty() && call_form(site, name, positions, benign, &tree) {
        return Ok(Verdict::CallForm);
    }

    if !walk.diffs.is_empty() {
        let shown = walk
            .diffs
            .iter()
            .take(6)
            .fold(String::new(), |mut out, diff| {
                let _ = write!(out, "\n    - {diff}");
                out
            });
        return Err(failure(
            site,
            hostile,
            format!(
                "the parse tree differs from the benign rendering's in {} place(s):{shown}\n  \
                 hostile: {}\n  benign:  {}",
                walk.diffs.len(),
                show(&sql),
                show(benign_sql)
            ),
        ));
    }
    Ok(Verdict::RoundTrip)
}

/// Render `site` with [`BENIGN`], parse it, and check the name lands exactly
/// at the positions the registry declares.
pub fn reference(site: &Site) -> Result<Reference, String> {
    reference_rendered(site, (site.render)(BENIGN))
}

/// [`reference`] over a benign rendering already produced.
pub fn reference_rendered(site: &Site, benign: Rendered) -> Result<Reference, String> {
    let sql = match benign {
        Rendered::Sql(sql) => sql,
        Rendered::Refused(err) => return Err(format!("refused the benign name: {err}")),
    };
    let tree = parse_tree(&sql).map_err(|err| {
        format!(
            "the benign rendering does not parse: {err}\n    {}",
            show(&sql)
        )
    })?;
    let mut positions = Vec::new();
    name_positions(&tree, &mut Vec::new(), &mut positions);
    let kinds: Vec<String> = positions.iter().map(|path| kind_of(path)).collect();
    if kinds != site.kinds {
        return Err(format!(
            "the registry declares the name at {:?}, but the benign rendering puts it at \
             {kinds:?}:\n    {}",
            site.kinds,
            show(&sql)
        ));
    }
    Ok(Reference {
        sql,
        tree,
        positions,
    })
}

/// Whether one of [`TYPE_SPELLINGS`] at a [`TypePart`](Policy::TypePart)
/// site read as a type keyword and nothing more: the trees agree everywhere
/// outside the `TypeName` node holding each position, and that node is still
/// a type name.
fn type_keyword(
    site: &Site,
    name: &str,
    positions: &[Vec<Seg>],
    benign: &Json,
    tree: &Json,
) -> bool {
    if site.policy != Policy::TypePart || !TYPE_SPELLINGS.contains(&name) {
        return false;
    }
    let mut regions = Vec::new();
    for position in positions {
        let Some(end) = position.iter().rposition(
            |seg| matches!(seg, Seg::Key(key) if key == "type_name" || key == "TypeName"),
        ) else {
            return false;
        };
        let region = position[..=end].to_vec();
        if !at(tree, &region).is_some_and(|node| node.get("names").is_some()) {
            return false;
        }
        regions.push(region);
    }
    let mut walk = Walk {
        name,
        policy: site.policy,
        path: Vec::new(),
        skip: regions,
        diffs: Vec::new(),
    };
    walk.compare(benign, tree);
    walk.diffs.is_empty()
}

/// Whether one of [`CALL_FORMS`] at a [`FunctionName`](Policy::FunctionName)
/// site read as the grammar's own expression and nothing more: at each
/// position the call node holding the name became the form's node, and the
/// trees agree everywhere outside it.
fn call_form(site: &Site, name: &str, positions: &[Vec<Seg>], benign: &Json, tree: &Json) -> bool {
    let Some((_, form)) = CALL_FORMS.iter().find(|(keyword, _)| *keyword == name) else {
        return false;
    };
    if site.policy != Policy::FunctionName {
        return false;
    }
    let mut regions = Vec::new();
    for position in positions {
        let Some(end) = position
            .iter()
            .rposition(|seg| matches!(seg, Seg::Key(key) if key == "FuncCall"))
        else {
            return false;
        };
        let region = position[..end].to_vec();
        if !at(tree, &region).is_some_and(|node| node.get(*form).is_some()) {
            return false;
        }
        regions.push(region);
    }
    let mut walk = Walk {
        name,
        policy: site.policy,
        path: Vec::new(),
        skip: regions,
        diffs: Vec::new(),
    };
    walk.compare(benign, tree);
    walk.diffs.is_empty()
}

/// The node at `path` in `tree`.
fn at<'a>(tree: &'a Json, path: &[Seg]) -> Option<&'a Json> {
    path.iter().try_fold(tree, |node, seg| match seg {
        Seg::Key(key) => node.get(key),
        Seg::Index(index) => node.get(index),
    })
}

/// A benign statement's [`Reference`] without the registry check — for a live
/// site whose statements are captured one by one, where the declared kinds
/// cover the whole sequence rather than any one statement.
pub fn reference_of(sql: &str) -> Result<Reference, String> {
    let tree = parse_tree(sql).map_err(|err| {
        format!(
            "the benign statement does not parse: {err}\n    {}",
            show(sql)
        )
    })?;
    let mut positions = Vec::new();
    name_positions(&tree, &mut Vec::new(), &mut positions);
    Ok(Reference {
        sql: sql.to_owned(),
        tree,
        positions,
    })
}

impl Reference {
    /// The kinds of the positions the name occupies, in walk order.
    pub fn kinds(&self) -> Vec<String> {
        self.positions.iter().map(|path| kind_of(path)).collect()
    }
}

/// Every path in `tree` whose leaf holds [`BENIGN`]: the name itself, or a
/// name the API derives from it (`pk-{table}`, `idx-{table}-{column}`), whose
/// hostile counterpart is the same derivation of the hostile name.
fn name_positions(tree: &Json, path: &mut Vec<Seg>, found: &mut Vec<Vec<Seg>>) {
    match tree {
        Json::Object(map) => {
            for (key, value) in map {
                path.push(Seg::Key(key.clone()));
                name_positions(value, path, found);
                path.pop();
            }
        }
        Json::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                path.push(Seg::Index(index));
                name_positions(value, path, found);
                path.pop();
            }
        }
        Json::String(text) if text.contains(BENIGN) => found.push(path.clone()),
        _ => {}
    }
}

/// Hold `site` to its declared NUL behaviour, returning the observed one.
pub fn judge_nul(site: &Site) -> Result<NulBehaviour, String> {
    let hostile = Hostile {
        label: "nul",
        name: NUL_NAME.to_owned(),
    };
    let declared = site.policy.nul();
    match (declared, (site.render)(NUL_NAME)) {
        (NulBehaviour::Refused, Rendered::Refused(err)) if err.contains("UnquotableIdentifier") => {
            Ok(NulBehaviour::Refused)
        }
        (NulBehaviour::Encoder, Rendered::Sql(sql)) if sql.contains('\0') => {
            encoder_refuses(&sql).map_err(|what| failure(site, &hostile, what))?;
            Ok(NulBehaviour::Encoder)
        }
        (NulBehaviour::Escaped, Rendered::Sql(sql)) if !sql.contains('\0') => {
            match parse_tree(&sql) {
                Err(err) if err.contains("invalid byte sequence") => Ok(NulBehaviour::Escaped),
                Err(err) => Err(failure(
                    site,
                    &hostile,
                    format!("the escaped NUL was refused for another reason: {err}"),
                )),
                Ok(_) => Err(failure(
                    site,
                    &hostile,
                    format!("the escaped NUL parses:\n    {}", show(&sql)),
                )),
            }
        }
        (declared, observed) => Err(failure(
            site,
            &hostile,
            format!("declared {declared:?}, observed {observed:?}"),
        )),
    }
}

/// Prove the protocol encoder refuses `sql` in both the extended-query
/// `Parse` message and the simple-query `Query` message — the only two routes
/// statement text takes to the server — and that libpg_query cannot parse it
/// either.
pub fn encoder_refuses(sql: &str) -> Result<(), String> {
    let mut buf = BytesMut::new();
    if frontend::parse("", sql, std::iter::empty(), &mut buf).is_ok() {
        return Err(format!("the Parse encoder accepted {}", show(sql)));
    }
    if frontend::query(sql, &mut buf).is_ok() {
        return Err(format!("the Query encoder accepted {}", show(sql)));
    }
    if pg_query::parse(sql, pg_query::ParserOptions::DEFAULT).is_ok() {
        return Err(format!("libpg_query parsed {}", show(sql)));
    }
    Ok(())
}

struct Walk<'a> {
    /// The hostile name.
    name: &'a str,
    policy: Policy,
    path: Vec<Seg>,
    /// Subtrees the comparison steps over.
    skip: Vec<Vec<Seg>>,
    diffs: Vec<String>,
}

impl Walk<'_> {
    fn diff(&mut self, what: String) {
        self.diffs
            .push(format!("at {}: {what}", path_text(&self.path)));
    }

    fn compare(&mut self, benign: &Json, hostile: &Json) {
        if self.skip.contains(&self.path) {
            return;
        }
        match (benign, hostile) {
            (Json::Object(benign), Json::Object(hostile)) => {
                let kinds = (benign.len() == 1, hostile.len() == 1);
                if kinds == (true, true) {
                    let (b, h) = (benign.keys().next(), hostile.keys().next());
                    if b != h && b.is_some_and(|k| k.starts_with(char::is_uppercase)) {
                        self.diff(format!(
                            "a {} node became a {} node",
                            b.map_or("", String::as_str),
                            h.map_or("", String::as_str)
                        ));
                        return;
                    }
                }
                for (key, b) in benign {
                    if POSITION_KEYS.contains(&key.as_str()) {
                        continue;
                    }
                    match hostile.get(key) {
                        Some(h) => {
                            self.path.push(Seg::Key(key.clone()));
                            self.compare(b, h);
                            self.path.pop();
                        }
                        None => self.diff(format!("`{key}` is missing")),
                    }
                }
                for key in hostile.keys() {
                    if !benign.contains_key(key) && !POSITION_KEYS.contains(&key.as_str()) {
                        self.diff(format!("`{key}` appears, holding {}", hostile[key]));
                    }
                }
            }
            (Json::Array(benign), Json::Array(hostile)) => {
                if benign.len() != hostile.len() {
                    self.diff(format!(
                        "{} element(s) where the benign tree has {}",
                        hostile.len(),
                        benign.len()
                    ));
                }
                for (index, (b, h)) in benign.iter().zip(hostile).enumerate() {
                    self.path.push(Seg::Index(index));
                    self.compare(b, h);
                    self.path.pop();
                }
            }
            (Json::String(b), Json::String(h)) if b.contains(BENIGN) => {
                if !self.policy.reads_back(b, self.name, h) {
                    let expected = self.policy.expected(b, self.name);
                    let what = format!("the name reads back as {h:?}, not {expected:?}");
                    self.diff(what);
                }
            }
            (b, h) if b != h => self.diff(format!("{h} where the benign tree has {b}")),
            _ => {}
        }
    }
}

/// The path of a node, for a failure message: keys dot-joined, indices in
/// brackets, and protobuf's `node` wrapper keys left out.
fn path_text(path: &[Seg]) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(key) if key == "node" => {}
            Seg::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Seg::Index(index) => {
                let _ = write!(out, "[{index}]");
            }
        }
    }
    out
}

/// A name position's kind: the innermost parse node that holds the name
/// (skipping value wrappers such as `String`), then the field path from that
/// node to the value — `RangeVar.relname`, `ColumnRef.fields[1]`,
/// `RangeVar.alias.aliasname`, `TypeName.names[0]`. A generic `List` is named
/// through the node holding it.
fn kind_of(path: &[Seg]) -> String {
    let is_node = |seg: &Seg| {
        matches!(seg, Seg::Key(key)
            if key.starts_with(char::is_uppercase) && !VALUE_WRAPPERS.contains(&key.as_str()))
    };
    let mut start = path.iter().rposition(is_node).unwrap_or(0);
    // A bare `List` says nothing about the position; name it by the node
    // that holds the list (`DropStmt.objects[0].List.items[0]`).
    if matches!(path.get(start), Some(Seg::Key(key)) if key == "List") {
        start = path[..start].iter().rposition(is_node).unwrap_or(start);
    }
    let mut out = String::new();
    for seg in &path[start..] {
        match seg {
            Seg::Key(key) if key == "node" || VALUE_WRAPPERS.contains(&key.as_str()) => {}
            Seg::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Seg::Index(index) => {
                let _ = write!(out, "[{index}]");
            }
        }
    }
    out
}
