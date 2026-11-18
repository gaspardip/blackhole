#[should_panic(expected = "missing")]
fn missing() { None::<u8>.expect("missing"); }
#[cfg_attr(feature = "a, allow(b)", derive(Debug))]
#[doc = "allow(unused)"]
struct Plain; // allow(x) expect(y)
fn call() { run(1, expect(2)); }
