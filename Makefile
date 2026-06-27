lint:
	cargo clippy

fmt:
	cargo fmt

# `cp -rf` can drag macOS `.DS_Store` files into dist/; strip them so they are
# never embedded by rust-embed or shipped in the crates.io package (Cargo's
# `include` globs would otherwise match them).
build-web:
	rm -rf dist \
	&& cd web \
	&& yarn install --network-timeout 600000 && yarn build \
	&& cp -rf dist ../ \
	&& find ../dist -name .DS_Store -delete

dev:
	cargo run -- redis:alpine?arch=amd64
dev-web:
	cargo watch -w src -x 'run -- --mode=web'
dev-docker:
	cargo run -- docker://redis:alpine
dev-ci:
	CI=true cargo run -- redis:alpine?arch=amd64	

udeps:
	cargo +nightly udeps

# 如果要使用需注释 profile.release 中的 strip
bloat:
	cargo bloat --release --crates

outdated:
	cargo outdated

release:
	cargo build --release
	ls -lh target/release

# Build the web UI, then publish to crates.io. `--allow-dirty` is required
# because `dist/` is .gitignore'd (untracked) yet bundled into the package
# via Cargo.toml's `include` key. Run a clean `make build-web` first so the
# real frontend — not build.rs's placeholder — ends up in the published crate.
publish: build-web
	cargo publish --allow-dirty

hooks:
	cp hooks/* .git/hooks/