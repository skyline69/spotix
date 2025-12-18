use wrapped_vec::WrappedVec;

use std::iter::FromIterator;

#[derive(WrappedVec)]
#[CollectionName = "Fruits"]
pub struct Fruit {}

#[test]
fn type_exists() {
    let _fruits: Fruits;
}

#[test]
fn implements_new() {
    let _fruits = Fruits::new();
}

#[test]
fn implements_is_empty() {
    assert!(Fruits::new().is_empty());
}

#[test]
fn implements_len() {
    assert_eq!(Fruits::new().len(), 0);
}

#[test]
fn implements_from_iterator() {
    let _fruits = Fruits::from_iter(vec![Fruit {}, Fruit {}]);
}

#[test]
fn implements_into_iterator() {
    let fruits = Fruits::new();
    for fruit in fruits.into_iter() {
        let _f: Fruit = fruit;
    }
}

#[test]
fn implements_into_iterator_ref() {
    let fruits = Fruits::new();
    for fruit in (&fruits).into_iter() {
        let _f: &Fruit = fruit;
    }
}

#[test]
fn implements_iter() {
    let fruits = Fruits::new();
    for _fruit in fruits.iter() {}
}

#[test]
fn implements_extend() {
    let mut fruits = Fruits::new();
    fruits.extend(vec![Fruit {}, Fruit {}]);
}

#[test]
fn implements_from_vec() {
    let _fruits = Fruits::from(vec![Fruit {}, Fruit {}]);
}

#[test]
fn implements_derives() {
    #[derive(Clone, Debug, WrappedVec)]
    #[CollectionName = "Fruits"]
    #[CollectionDerives = "Clone, Debug"]
    pub struct Fruit {};

    let _debug = format!("{:?}", Fruit {});
    let _clone = (Fruit {}).clone();
}
