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

define bump_version
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "Error: working tree is not clean; commit or stash changes first." >&2; \
		exit 1; \
	fi
	@NEXT=$$(python3 -c 'import re, sys; \
		txt = open("Cargo.toml").read(); \
		m = re.search(r"^version\s*=\s*\"(\d+)\.(\d+)\.(\d+)\"", txt, re.M); \
		maj, min_, pat = map(int, m.groups()); \
		kind = "$(1)"; \
		ver = f"{maj}.{min_}.{pat+1}" if kind == "patch" else f"{maj}.{min_+1}.0" if kind == "minor" else f"{maj+1}.0.0"; \
		open("Cargo.toml", "w").write(txt[:m.start()] + f"version = \"{ver}\"" + txt[m.end():]); \
		print(ver)') \
	&& cargo update -p diving --offline >/dev/null \
	&& git add Cargo.toml Cargo.lock \
	&& git commit -m "chore: version $$NEXT" \
	&& git tag "v$$NEXT" \
	&& echo "Bumped to $$NEXT (tag v$$NEXT). Run 'git push && git push --tags' to publish."
endef

.PHONY: version-patch version-minor version-major

version-patch:
	$(call bump_version,patch)

version-minor:
	$(call bump_version,minor)

version-major:
	$(call bump_version,major)