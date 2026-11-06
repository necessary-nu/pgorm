//! Making a projection addressable, so a deduplication can key on all of it.
//!
//! prqlc resolves `this` — the whole-relation reference
//! [`distinct`](super::Pipeline::distinct) deduplicates on — by walking a
//! namespace keyed by name. Two ordinary projections have columns that
//! namespace cannot hold: an expression left unnamed is filed nowhere at all,
//! and of two columns competing for one name only the last is filed. Either
//! way the column is missing from the deduplication key *and* from the
//! result, because the key is what the deduplicated relation projects.
//!
//! Naming is what makes a column addressable, so this module decides which
//! items need a name of their own and what to call them — and, because a
//! settled relation answers under the binding's name and no longer under the
//! sources' own, how a reference written against a source follows the column
//! it names across that boundary.
//!
//! One column resists naming altogether: the wildcard a relation whose schema
//! prqlc cannot see still projects. It has no name to mint over, so
//! [`bare_star_key`] answers the other question instead — whether the key it
//! lands in can be written down at all.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::adapter::{self, PlExpr};

/// The prefix a minted name carries: distinct from prqlc's own `_expr_N` so
/// the two sequences cannot collide, and outside the reserved set
/// [`into_sql`](super::Pipeline::into_sql) screens for.
const MINTED: &str = "_col_";

/// What making a projection addressable changed.
// [spec:pgorm:req:pipeline.compose+1]
#[derive(Debug, Default)]
pub(super) struct Naming {
    /// Whether any item gained a name it did not carry before.
    ///
    /// Renaming moves a column from the submodule of the source that
    /// qualified it to the top level of the namespace, which is a different
    /// column shape than the one the pipeline has been tracking. Settling at
    /// a binding boundary re-exposes the whole projection as one input's own
    /// columns — the shape that needs no tracking at all — so a renamed
    /// projection settles rather than having its shape re-derived.
    pub(super) renamed: bool,
    /// Where a column ended up, keyed by the dotted path it had before:
    /// what a sort key naming that column has to follow.
    pub(super) exposed: HashMap<String, String>,
}

/// Give every column the trailing projection stages introduce a name of its
/// own, rewriting those stages in place.
///
/// Only the stages back to the last one that replaced the relation's columns
/// are considered: those are the ones whose items are still the projection.
/// Of two items competing for a name the *later* keeps it, which matches both
/// prqlc's namespace (the last filed wins) and its rendering (the earlier is
/// the one it renames), so a reference written against the surviving column
/// still means what it meant.
///
/// A `join` between the projection and here adds columns that are not read
/// off any stage, so a collision with the far side is not caught; within what
/// a projection lists, every collision is.
// [spec:pgorm:req:pipeline.compose+1]
pub(super) fn disambiguate(stages: &mut [PlExpr]) -> Naming {
    let projections = projection_stages(stages);
    let mut names: Vec<Option<String>> = Vec::new();
    for &index in &projections {
        let Some(items) = adapter::tuple_items_mut(&mut stages[index]) else {
            continue;
        };
        names.extend(
            items
                .iter()
                .map(|item| adapter::exposed_name(item).map(str::to_owned)),
        );
    }

    let minted = mint(&names);
    if minted.is_empty() {
        return Naming::default();
    }

    let mut naming = Naming {
        renamed: true,
        exposed: HashMap::new(),
    };
    let mut position = 0;
    for &index in &projections {
        let items = {
            let Some(items) = adapter::tuple_items_mut(&mut stages[index]) else {
                continue;
            };
            std::mem::take(items)
        };
        let renamed = items
            .into_iter()
            .map(|item| {
                let at = position;
                position += 1;
                let Some(name) = minted.get(&at) else {
                    return item;
                };
                if let Some(path) = adapter::column_path(&item) {
                    naming.exposed.insert(path, name.clone());
                }
                adapter::aliased(item, name.clone())
            })
            .collect();
        if let Some(items) = adapter::tuple_items_mut(&mut stages[index]) {
            *items = renamed;
        }
    }
    naming
}

/// Whether deduplicating these stages would key on a star with no legal
/// spelling.
///
/// `starred` says the relation still projects a wildcard, which `this` expands
/// into the deduplication key alongside the named columns. A wildcard is
/// renderable there only while prqlc qualifies it: `cake.*` is a whole-row
/// reference PostgreSQL reads as a composite value, and a bare `*` is not an
/// expression its grammar has at all. prqlc qualifies by table exactly when
/// the query it is rendering reads more than one — `omit_ident_prefix` is set
/// for a single table and nothing else — so counting the relations in scope
/// answers it.
///
/// The count is of the stages that *bring* a relation, `from` and `join`,
/// because those are the ones prqlc renders as tables of the same query; a
/// stage appended after the deduplication belongs to a query that has already
/// read from this one.
// [spec:pgorm:req:pipeline.compose+1]
pub(super) fn bare_star_key(stages: &[PlExpr], starred: bool) -> bool {
    starred
        && stages
            .iter()
            .filter(|stage| matches!(adapter::stage_verb(stage), Some("from" | "join")))
            .count()
            == 1
}

/// The source names a settle took out of scope, and where their columns went.
///
/// Settling moves the stages into a `let` binding and leaves the pipeline
/// reading from it, so prqlc resolves the relation's columns under that
/// binding's name and under no other. Every reference a *later* stage writes
/// against a source — `col(source, column)`, an entity column — would name a
/// namespace that is no longer there, and prqlc refuses the pipeline before
/// any SQL is emitted. Those references are repointed on the way in, which is
/// the only direction available: the pipeline is built forward, so at the
/// moment of settling the stages that will refer to the sources do not exist
/// yet.
///
/// The binding qualifies the repointed reference rather than the reference
/// being left bare. It costs nothing — prqlc renders a single input's column
/// unqualified either way — and it stays unambiguous when a later `join`
/// brings a second relation alongside, where a bare name could mean either.
// [spec:pgorm:req:pipeline.compose+1]
#[derive(Debug, Clone, Default)]
pub(super) struct Settled {
    /// The binding the relation reads from, or `None` while nothing has been
    /// settled and every source still answers for itself.
    binding: Option<String>,
    /// Source names that resolved before the settle and do not after.
    shadowed: BTreeSet<String>,
    /// Where a column that had to be renamed to stay addressable ended up,
    /// keyed by the dotted path it had — [`Naming::exposed`], kept past the
    /// settle that consumed it because a later stage may name it too.
    exposed: HashMap<String, String>,
}

impl Settled {
    /// Record a settle: the sources the moved `stages` read are out of scope
    /// from here, and the relation answers under `binding`.
    ///
    /// A set operation settles with nothing renamed, so `naming` is that
    /// pipeline's [`Naming::default`]; a deduplication hands over the one it
    /// made the projection addressable with.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) fn absorb(&mut self, stages: &[PlExpr], binding: &str, naming: &Naming) {
        for stage in stages {
            if !matches!(adapter::stage_verb(stage), Some("from" | "join")) {
                continue;
            }
            if let Some(source) = adapter::stage_source(stage) {
                self.shadowed.insert(source.to_owned());
            }
        }
        for (path, name) in &naming.exposed {
            self.exposed.insert(path.clone(), name.clone());
        }
        self.binding = Some(binding.to_owned());
    }

    /// A relation read again — the operand of a `join` — answers for itself
    /// once more, whatever an earlier settle did to a source of that name.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) fn rejoined(&mut self, reference: &PlExpr) {
        if let Some(source) = adapter::exposed_name(reference) {
            self.shadowed.remove(source);
        }
    }

    /// Repoint every reference in `node` that names a shadowed source at the
    /// binding that now exposes its column.
    // [spec:pgorm:req:pipeline.compose+1]
    pub(super) fn requalify(&self, node: &mut PlExpr) {
        if self.shadowed.is_empty() {
            return;
        }
        adapter::requalify(node, &|path| self.repoint(path));
    }

    /// Where a reference written against a source points now: the binding,
    /// and the name the column answers to there — the one it was renamed to
    /// if it had to be, and otherwise its own.
    fn repoint(&self, path: &str) -> Option<(String, String)> {
        let binding = self.binding.as_ref()?;
        let (source, column) = path.split_once('.')?;
        if !self.shadowed.contains(source) {
            return None;
        }
        let exposed = match self.exposed.get(path) {
            Some(renamed) => renamed.clone(),
            None => column.to_owned(),
        };
        Some((binding.clone(), exposed))
    }
}

/// A sort stage repointed at the names a settled binding exposes.
///
/// Behind a CTE the relation's columns answer to their own bare names and
/// nothing else, so a key still qualified by the source it came from no
/// longer resolves, and a key whose column was renamed has to follow it.
/// `None` when some key is not a column reference — there is no bare name to
/// find for an ordering computed from an expression, and leaving the order as
/// it was beats emitting SQL prqlc refuses. That judgement is why this is not
/// [`Settled::requalify`]: the ordering is restated *behind* the deduplicating
/// group, which has re-exposed the relation under names of its own, and a key
/// that cannot follow is dropped rather than rewritten. A key this leaves bare
/// names no source, so the general repointing passes over it untouched.
// [spec:pgorm:req:pipeline.compose+1]
pub(super) fn rebound_sort(mut sort: PlExpr, naming: &Naming) -> Option<PlExpr> {
    let keys = adapter::tuple_items_mut(&mut sort)?;
    for key in keys.iter_mut() {
        let path = adapter::sort_key_path(key)?;
        let exposed = match naming.exposed.get(&path) {
            Some(minted) => minted.clone(),
            None => path.rsplit('.').next()?.to_owned(),
        };
        if !adapter::rebind_sort_key(key, &exposed) {
            return None;
        }
    }
    Some(sort)
}

/// Which positions need a name minted, and what to call them: the ones left
/// unnamed, and the ones a later item takes the name of.
///
/// A minted name is checked against every name already in play and against
/// the ones minted before it, so a projection that happens to carry a column
/// spelled like one of these is stepped around rather than collided with.
fn mint(names: &[Option<String>]) -> HashMap<usize, String> {
    let mut taken: HashSet<String> = names.iter().flatten().cloned().collect();
    let mut minted = HashMap::new();
    for (position, name) in names.iter().enumerate() {
        let unaddressable = match name {
            None => true,
            Some(name) => names[position + 1..]
                .iter()
                .flatten()
                .any(|later| later == name),
        };
        if !unaddressable {
            continue;
        }
        let mut suffix = position;
        let mut candidate = format!("{MINTED}{suffix}");
        while taken.contains(&candidate) {
            suffix += 1;
            candidate = format!("{MINTED}{suffix}");
        }
        taken.insert(candidate.clone());
        minted.insert(position, candidate);
    }
    minted
}

/// The indices of the stages whose items are still the relation's own
/// projection: every `select` and `derive` back to, and including, the last
/// stage that replaced the relation's columns.
///
/// The walk stops at anything whose columns cannot be read off the stage
/// itself — a source's own, an aggregate's, a set operation's — and leaves
/// what it did not reach alone, so stopping early costs completeness and
/// never correctness.
fn projection_stages(stages: &[PlExpr]) -> Vec<usize> {
    let mut found = Vec::new();
    for (index, stage) in stages.iter().enumerate().rev() {
        match adapter::stage_verb(stage) {
            Some("select") => {
                found.push(index);
                break;
            }
            Some("derive") => found.push(index),
            Some("from" | "group" | "append" | "intersect" | "remove") => break,
            _ => {}
        }
    }
    found.reverse();
    found
}
