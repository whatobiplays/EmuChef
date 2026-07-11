# Frozen v1 Compatibility Fixtures

The files in this directory capture EmuChef's version 1 compatibility
contract. Rust tests consume them as immutable expected results.

These fixtures are not regenerated from Python. Changing an existing file
requires an explicit decision to revise the compatibility contract. Tests for
new functionality must use Rust-native fixtures and expectations outside this
directory.
