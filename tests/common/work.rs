/// # Panics
/// Panics if the work exceeds the per-byte cap.
#[track_caller]
pub fn assert_work(work: u64, bytes: usize, units_per_byte: u64, label: &str) {
    let cap = bytes as u64 * units_per_byte;
    assert!(work <= cap, "{label}: {work} work exceeds {cap}");
}

/// # Panics
/// Panics if work grows past the given factor.
#[track_caller]
pub fn assert_growth(small: u64, large: u64, factor: u64, label: &str) {
    assert!(
        large <= small * factor,
        "work grew from {small} to {large} (limit {factor}x) {label}"
    );
}
