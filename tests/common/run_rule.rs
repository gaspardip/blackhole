use blackhole_check::engine::{Finding, registry, run};

pub fn run_rule(id: &str, name: &str, source: &str) -> Vec<Finding> {
    let rule = registry()
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("missing rule {id}"));
    run(std::slice::from_ref(rule), [(name, source.as_bytes())])
}
