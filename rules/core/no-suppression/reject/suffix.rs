#![deny(dead_code)]
#[cfg_attr(all(), allow(dead_code), doc = stringify!("foo"r"bar\"tail"))]
fn covered() {}
