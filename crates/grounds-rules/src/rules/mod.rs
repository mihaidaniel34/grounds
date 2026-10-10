//! Rule implementations grouped by theme.

pub mod equality;

use crate::framework::Rule;

pub fn all() -> Vec<Box<dyn Rule>> {
    let mut v: Vec<Box<dyn Rule>> = Vec::new();
    v.extend(equality::rules());
    v
}
