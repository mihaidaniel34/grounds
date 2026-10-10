//! Rule implementations grouped by theme.

pub mod api;
pub mod concurrency;
pub mod equality;
pub mod exceptions;
pub mod numbers;
pub mod strings;

use crate::framework::Rule;

pub fn all() -> Vec<Box<dyn Rule>> {
    let mut v: Vec<Box<dyn Rule>> = Vec::new();
    v.extend(equality::rules());
    v.extend(numbers::rules());
    v.extend(strings::rules());
    v.extend(exceptions::rules());
    v.extend(concurrency::rules());
    v.extend(api::rules());
    v
}
