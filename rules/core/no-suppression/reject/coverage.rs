#[cfg_attr(tarpaulin, ignore)]
#[test]
fn example() {}
#[cfg(not(all(tarpaulin)))]
fn other() {}
