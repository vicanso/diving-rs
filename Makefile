lint:
	cargo clippy

fmt:
	cargo fmt

build-web:
	cd web \
	&& yarn install --network-timeout 600000 && yarn build \
	&& cp -rf dist ../

dev:
	cargo run -- redis:alpine?arch=amd64
dev-web:
	cargo watch -w src -x 'run -- --mode=web'
dev-docker:
	cargo run -- docker://redis:alpine

udeps:
	cargo +nightly udeps

release:
	cargo build --release
	ls -lh target/release

hooks:
	cp hooks/* .git/hooks/