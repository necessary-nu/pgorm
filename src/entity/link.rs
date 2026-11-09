use crate::{EntityTrait, QuerySelect, Related, RelationDef, Select};
use pgorm_query::{JoinType, SqlName};
use std::{
    fmt::{self, Debug},
    marker::PhantomData,
};

/// The name a linked join binds one hop's table to.
///
/// A [`Linked`] chain aliases its hops positionally — `r0`, `r1`, … — because
/// the same table may appear at more than one hop and an unaliased self-join
/// is ambiguous. Those names are generated, but they are visible: a caller
/// ordering or grouping by a column of a joined table has to name it. So the
/// name is handed out as a value, derived by [`Linked::last_hop_alias`] from
/// the same chain the join walks, instead of being retyped at the call site —
/// where a chain that gained a hop would silently rebind the string to a
/// different table.
///
/// It is an ordinary [`SqlName`], so it stands in any identifier position:
///
/// ```
/// use pgorm::{Linked, QueryOrder, QueryTrait, RelatedLink};
/// use pgorm::tests_cfg::{cake, fruit};
/// use pgorm_query::Expr;
///
/// let link = RelatedLink::<cake::Entity, fruit::Entity>::new();
/// let sql = link
///     .find_linked()
///     .order_by_asc(Expr::col((link.last_hop_alias(), cake::Column::Id)))
///     .as_query()
///     .to_string();
///
/// assert!(sql.ends_with(r#"ORDER BY "r0"."id" ASC"#), "{sql}");
/// ```
// [spec:pgorm:req:entity.relation.linked+4]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkedAlias(usize);

impl LinkedAlias {
    /// The alias bound to the `i`-th hop of a link, counting from zero.
    pub const fn hop(i: usize) -> Self {
        Self(i)
    }
}

// [spec:pgorm:req:entity.relation.linked+4]
impl SqlName for LinkedAlias {
    fn unquoted(&self, s: &mut dyn fmt::Write) {
        let _ = write!(s, "r{}", self.0);
    }
}

impl fmt::Display for LinkedAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// A Trait for links between Entities
// [spec:pgorm:req:entity.relation.linked+4]
pub trait Linked {
    #[allow(missing_docs)]
    type FromEntity: EntityTrait;

    #[allow(missing_docs)]
    type ToEntity: EntityTrait;

    /// Link for an Entity
    fn link(&self) -> Vec<RelationDef>;

    /// The alias bound while the chain's last relation is joined.
    ///
    /// [`find_linked`] walks the ladder backwards from `ToEntity`, so the last
    /// rung is the *source* table — which is why it is what scopes
    /// [`ModelTrait::find_linked`](crate::ModelTrait::find_linked) to one
    /// model, and what a caller ordering or filtering on a `FromEntity` column
    /// has to name.
    ///
    /// The builder that walks the chain derives the name here, and so should a
    /// caller, rather than writing `r3` and hoping the chain never grows.
    ///
    /// [`find_linked`]: Linked::find_linked
    fn last_hop_alias(&self) -> LinkedAlias {
        LinkedAlias::hop(self.link().len().saturating_sub(1))
    }

    /// Find all the Entities that are linked to the Entity
    ///
    /// The chain is walked backwards from `ToEntity`, so each hop is joined in
    /// the reverse of the direction its [`RelationDef`] was written in — which
    /// is [`QuerySelect::join_as_rev`], the crate's one reverse edge. Walking
    /// is therefore only a matter of binding the two ends of each hop: the
    /// joined side takes this hop's alias, and the far side is re-pointed at
    /// the alias the previous hop bound it under, the innermost hop's far side
    /// being the target table the statement already selects from.
    ///
    /// Because the ON clause is then the ordinary one every other join in the
    /// crate emits, a hop honours everything its relation declares — its
    /// `condition_type` and its `on_condition`, the closure receiving the two
    /// aliases in the roles the relation was written with.
    fn find_linked(&self) -> Select<Self::ToEntity> {
        let mut select = Select::new();
        for (i, mut rel) in self.link().into_iter().rev().enumerate() {
            if i > 0 {
                rel.to_tbl = rel.to_tbl.alias(LinkedAlias::hop(i - 1));
            }
            select = select.join_as_rev(JoinType::InnerJoin, rel, LinkedAlias::hop(i));
        }
        select
    }
}

/// The [`Linked`] chain a [`Related`] implementation already describes.
///
/// A one-hop link is the relation itself, and a junction-mediated one is
/// `[via, to]` — exactly what `Related` carries. Writing a unit struct plus an
/// eleven-line `Linked` impl to restate that is duplication, so hand this to
/// [`ModelTrait::find_linked`](crate::ModelTrait::find_linked) instead.
///
/// The source entity is inferred from the position it is used in; only the
/// target is named:
///
/// ```
/// use pgorm::{ModelTrait, QueryTrait, RelatedLink, tests_cfg::{cake, fruit}};
///
/// let cake = cake::Model { id: 12, name: "Cheesecake".to_owned() };
///
/// assert_eq!(
///     cake.find_linked(RelatedLink::to(fruit::Entity))
///         .as_query()
///         .to_string(),
///     [
///         r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id""#,
///         r#"FROM "fruit""#,
///         r#"INNER JOIN "cake" AS "r0" ON "r0"."id" = "fruit"."cake_id""#,
///         r#"WHERE "r0"."id" = 12"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// The linked form aliases the joined table, so it is also what a self-relation
/// needs: `RelatedLink::to(Entity)` on an entity related to itself joins the
/// table a second time under `r0` rather than emitting an ambiguous self-join.
// [spec:pgorm:req:entity.relation.linked+4]
pub struct RelatedLink<E, R>(PhantomData<fn() -> (E, R)>);

impl<E, R> RelatedLink<E, R> {
    /// The link to the given entity, through whichever [`Related`]
    /// implementation connects it to the entity being selected from.
    pub fn to(_: R) -> Self {
        Self(PhantomData)
    }

    /// The link, with the target entity named as a type rather than a value.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E, R> Default for RelatedLink<E, R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E, R> Clone for RelatedLink<E, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E, R> Copy for RelatedLink<E, R> {}

impl<E, R> Debug for RelatedLink<E, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RelatedLink")
    }
}

// [spec:pgorm:req:entity.relation.linked+4]
impl<E, R> Linked for RelatedLink<E, R>
where
    E: EntityTrait + Related<R>,
    R: EntityTrait,
{
    type FromEntity = E;

    type ToEntity = R;

    fn link(&self) -> Vec<RelationDef> {
        match <E as Related<R>>::via() {
            Some(via) => vec![via, <E as Related<R>>::to()],
            None => vec![<E as Related<R>>::to()],
        }
    }
}

// [spec:pgorm:req:entity.relation.linked+4/test]    walking a chain is only the
// binding of each hop's two ends, the join itself being the crate's reverse
// edge — so a hop carries the whole relation, `condition_type` included
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_cfg::{cake, cake_filling, entity_linked, filling};
    use crate::{QueryTrait, RelationTrait};
    use pgorm_query::{ConditionType, Expr, ExprTrait, IntoCondition};
    use pretty_assertions::assert_eq;

    #[test]
    fn a_chain_walk_is_the_reverse_edge_twice() {
        // The same two hops written by hand: each relation's source side
        // re-aliased as this hop's rung, its far side re-pointed at the rung
        // the previous hop bound — and nothing else.
        let mut inner = cake_filling::Relation::Cake.def().rev();
        inner.to_tbl = inner.to_tbl.alias(LinkedAlias::hop(0));

        let by_hand = Select::<filling::Entity>::new()
            .join_as_rev(
                JoinType::InnerJoin,
                cake_filling::Relation::Filling.def(),
                LinkedAlias::hop(0),
            )
            .join_as_rev(JoinType::InnerJoin, inner, LinkedAlias::hop(1));

        assert_eq!(
            entity_linked::CakeToFilling
                .find_linked()
                .as_query()
                .to_string(),
            by_hand.as_query().to_string()
        );
    }

    /// A hop whose relation combines its ON clauses with `OR`. Nothing in
    /// `tests_cfg` links through one, and the walk used to drop the setting on
    /// the floor by always building `Condition::all()`.
    #[derive(Debug)]
    struct OrLink;

    impl Linked for OrLink {
        type FromEntity = cake_filling::Entity;
        type ToEntity = filling::Entity;

        fn link(&self) -> Vec<RelationDef> {
            vec![
                cake_filling::Relation::Filling
                    .def()
                    .condition_type(ConditionType::Any)
                    .on_condition(|left, _right| {
                        Expr::col((left, cake_filling::Column::CakeId))
                            .gt(10)
                            .into_condition()
                    }),
            ]
        }
    }

    #[test]
    fn a_hop_combines_on_clauses_per_the_relation() {
        assert_eq!(
            OrLink.find_linked().as_query().to_string(),
            [
                r#"SELECT "filling"."id", "filling"."name", "filling"."vendor_id""#,
                r#"FROM "filling""#,
                r#"INNER JOIN "cake_filling" AS "r0""#,
                r#"ON "r0"."filling_id" = "filling"."id" OR "r0"."cake_id" > 10"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn the_last_rung_is_the_source_table() {
        let link = RelatedLink::<cake::Entity, filling::Entity>::new();
        assert_eq!(link.last_hop_alias(), LinkedAlias::hop(1));

        let sql = link.find_linked().as_query().to_string();
        assert!(
            sql.contains(r#"INNER JOIN "cake" AS "r1" ON "r1"."id" = "r0"."cake_id""#),
            "{sql}"
        );
    }
}
