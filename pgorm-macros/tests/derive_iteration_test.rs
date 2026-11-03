//! The iteration the derives write: `Relation`, `Column` and `ActiveEnum`
//! make the enums they are given iterable, and the composite derive the
//! `Column` and `PrimaryKey` it makes, so no program derives it.

use pgorm::entity::prelude::*;
use pgorm::tests_cfg::{active_enums::Tea, cake, cake_filling};

#[derive(Copy, Clone, Debug, PartialEq, Column)]
enum Simple {
    A,
    B,
    C,
}

// [spec:pgorm:sem:macros.derive.enum-iter+1/test]    the `{Enum}Iter` struct each derive writes, and `Iterable`
// [spec:pgorm:sem:macros.derive.boilerplate/test]    `Relation`, `Column`, `ActiveEnum` and the composite's enums iterate with no `EnumIter`
#[test]
fn each_derive_makes_its_enum_iterable() {
    let relations: cake::RelationIter = cake::Relation::iter();
    assert_eq!(
        relations
            .map(|relation| format!("{relation:?}"))
            .collect::<Vec<_>>(),
        ["Fruit", "TropicalFruit", "OrTropicalFruit"]
    );
    let simple: SimpleIter = Simple::iter();
    assert_eq!(
        simple.collect::<Vec<_>>(),
        [Simple::A, Simple::B, Simple::C]
    );
    assert_eq!(
        Tea::iter().collect::<Vec<_>>(),
        [Tea::EverydayTea, Tea::BreakfastTea]
    );
    assert_eq!(
        cake::Column::iter()
            .map(|column| column.to_string())
            .collect::<Vec<_>>(),
        ["id", "name"]
    );
    assert_eq!(
        cake_filling::PrimaryKey::iter()
            .map(|key| key.as_str().to_owned())
            .collect::<Vec<_>>(),
        ["cake_id", "filling_id"]
    );

    fn assert_iterable<E: Iterable>() {}
    assert_iterable::<cake::Relation>();
    assert_iterable::<cake::Column>();
    assert_iterable::<cake::PrimaryKey>();
    assert_iterable::<Tea>();
}

// [spec:pgorm:sem:macros.derive.enum-iter+1/test]    the trait set on the iterator
#[test]
fn the_iterator_implements_the_full_trait_set() {
    fn assert_fused<I: std::iter::FusedIterator>(_: &I) {}

    let iter = Simple::iter();
    assert_fused(&iter);

    assert_eq!(iter.len(), 3);
    let cloned = iter.clone();
    assert_eq!(cloned.count(), 3);
    assert_eq!(format!("{:?}", Simple::iter()), "SimpleIter { len: 3 }");
    assert_eq!(
        Simple::iter().rev().collect::<Vec<_>>(),
        [Simple::C, Simple::B, Simple::A]
    );

    let mut done = Simple::iter();
    for _ in 0..3 {
        assert!(done.next().is_some());
    }
    assert!(done.next().is_none());
    assert!(done.next().is_none());

    let mut both = Simple::iter();
    assert_eq!(both.next(), Some(Simple::A));
    assert_eq!(both.next_back(), Some(Simple::C));
    assert_eq!(both.len(), 1);
    assert_eq!(both.next(), Some(Simple::B));
    assert_eq!(both.next_back(), None);
}
