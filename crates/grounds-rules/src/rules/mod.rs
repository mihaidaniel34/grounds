//! Rule implementations grouped by theme.

pub mod api;
pub mod classes;
pub mod complexity;
pub mod concurrency;
pub mod control;
pub mod datetime;
pub mod di;
pub mod equality;
pub mod exceptions;
pub mod flow;
pub mod numbers;
pub mod resolution;
pub mod security;
pub mod strings;
pub mod testing;
pub mod unused;

use crate::framework::Rule;

pub fn all() -> Vec<Box<dyn Rule>> {
    let mut v: Vec<Box<dyn Rule>> = Vec::new();
    v.extend(equality::rules());
    v.extend(numbers::rules());
    v.extend(strings::rules());
    v.extend(exceptions::rules());
    v.extend(concurrency::rules());
    v.extend(api::rules());
    v.extend(control::rules());
    v.extend(classes::rules());
    v.extend(unused::rules());
    v.extend(flow::rules());
    v.extend(security::rules());
    v.extend(datetime::rules());
    v.extend(testing::rules());
    v.extend(complexity::rules());
    v.extend(di::rules());
    v.extend(resolution::rules());
    v
}
