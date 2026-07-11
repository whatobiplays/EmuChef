#[test]
fn step_specs_are_defined_in_rust_source() {
    let source = include_str!("../src/step_specs.rs");

    assert!(
        !source.contains("include_str!"),
        "StepSpec metadata must be built from Rust-owned data, not an external JSON fixture"
    );
}
