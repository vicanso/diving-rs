lint:
	cargo clippy

fmt:
	cargo fmt

dev:
	cargo run -- mongo

release:
	cargo build --release