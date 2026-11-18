#[should_panic(expected = "missing")]
fn missing() { None::<u8>.expect("missing"); }
