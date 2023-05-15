lint:
	cargo clippy

fmt:
	cargo fmt

build-web:
	cd web \
	&& yarn install && yarn build \
	&& cp -rf dist ../

dev:
	cargo run -- redis:alpine
dev-web:
	cargo watch -w src -x 'run -- --mode=web'

udeps:
	cargo +nightly udeps

release:
	cargo build --release
	ls -lh target/release

hooks:
	cp hooks/* .git/hooks/