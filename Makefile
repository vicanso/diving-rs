lint:
	cargo clippy

fmt:
	cargo fmt

build-web:
	cd web \
	&& yarn install && yarn build \
	&& cp -rf dist ../

dev:
	cargo run -- mongo
dev-web:
	cargo watch -w src -x 'run -- --mode=web'

release:
	cargo build --release
	ls -lh target/release