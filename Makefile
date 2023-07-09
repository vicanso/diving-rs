lint:
	cargo clippy

fmt:
	cargo fmt

build-web:
	rm -rf dist \
	&& cd web \
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

# 如果要使用需注释 profile.release 中的 strip
bloat:
	cargo bloat --release --crates

release:
	cargo build --release
	ls -lh target/release

hooks:
	cp hooks/* .git/hooks/