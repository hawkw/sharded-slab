use crate::{
    page::{self, Page},
    sync::CausalCell,
    Shard,
};
use std::slice;
pub struct Iter<'a, T> {
    shards: slice::Iter<'a, CausalCell<Shard<T>>>,
    pages: slice::Iter<'a, Page<T>>,
    slots: page::Iter<'a, T>,
}
