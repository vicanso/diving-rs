lint:
	cargo clippy

fmt:
	cargo fmt

dev:
	cargo run -- mongo
dev-web:
	cargo watch -w src -x 'run -- --mode=web'

release:
	cargo build --release