# Tests embed the interpreter that built the crate and import whenever from
# the checkout next to this one. Point PYO3_PYTHON at a Python that has the
# whenever extension built in place (`make build` there).
PYO3_PYTHON ?= $(abspath ../whenever/.venv/bin/python)
export PYO3_PYTHON

.PHONY: test
test:
	cargo test --all-features
	PYO3_WHENEVER_TEST_PURE=1 cargo test --all-features

.PHONY: lint
lint:
	cargo fmt -- --check
	cargo clippy --all-targets --all-features -- -D warnings

.PHONY: fix
fix:
	cargo fmt
