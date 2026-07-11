# Compatibility Fixtures

`crates/emuchef-rust-backend/tests/fixtures/compatibility_goldens_v1` contains
frozen results that protect the version 1 data and behavior contract.

The files are immutable and are not regenerated from Python. A change requires
an explicit compatibility-contract decision and review of every affected
consumer. New features should add Rust-native fixtures or direct assertions
outside this directory.
