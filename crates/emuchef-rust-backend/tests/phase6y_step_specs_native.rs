#[test]
fn step_specs_are_rust_native_not_python_fixture_backed() {
    let source = include_str!("../src/step_specs.rs");

    assert!(
        !source.contains("python_step_specs"),
        "Rust StepSpec metadata must not embed the Python-generated fixture"
    );
    assert!(
        !source.contains("PYTHON_STEP_SPECS"),
        "Rust StepSpec metadata must not retain Python fixture constants"
    );
    assert!(
        !source.contains("include_str!"),
        "Rust StepSpec metadata must be built from Rust-owned data, not an external JSON fixture"
    );
}
