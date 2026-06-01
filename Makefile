.PHONY: dev fmt lint test ci

dev:
	./scripts/dev.sh

fmt:
	cd rust && cargo fmt --all

lint:
	cd rust && cargo clippy --workspace --all-targets -- -D warnings

test:
	cd rust && cargo test --workspace

ci:
	cd rust && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
