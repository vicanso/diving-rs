lint:
	cargo clippy

fmt:
	cargo fmt

dev:
	cargo run -- mongo
dev-web:
	cargo run -- --mode=web

release:
	cargo build --release