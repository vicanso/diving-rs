lint:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

test:
	cargo test --workspace

# `cp -rf` can drag macOS `.DS_Store` files into `crates/diving-web/dist/`;
# strip them so they are never embedded by rust-embed.
build-web:
	rm -rf crates/diving-web/dist \
	&& cd web \
	&& yarn install --network-timeout 600000 && yarn build \
	&& cp -rf dist ../crates/diving-web/dist \
	&& find ../crates/diving-web/dist -name .DS_Store -delete

build-lib:
	cargo build --release -p diving-core

build-bin:
	cargo build --release -p diving

build-web-bin: build-web
	cargo build --release -p diving-web

release-web: build-web-bin

release: build-bin build-web-bin

dev:
	cargo run -p diving -- redis:alpine?arch=amd64

dev-web:
	cargo watch -w crates -x 'run -p diving-web --'

dev-docker:
	cargo run -p diving -- docker://redis:alpine

dev-ci:
	CI=true cargo run -p diving -- redis:alpine?arch=amd64

udeps:
	cargo +nightly udeps

# 如果要使用需注释 profile.release 中的 strip
bloat:
	cargo bloat --release -p diving --crates

outdated:
	cargo outdated

# Publish publishable workspace crates in dependency order.
# `diving-core` and `diving` do not bundle web assets, so no `build-web`
# or `--allow-dirty` is needed.
publish:
	cargo publish -p diving-core \
	&& sleep 15 \
	&& cargo publish -p diving-app \
	&& sleep 15 \
	&& cargo publish -p diving

hooks:
	cp hooks/* .git/hooks/

define bump_version
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "Error: working tree is not clean; commit or stash changes first." >&2; \
		exit 1; \
	fi
	@NEXT=$$(python3 -c 'import re; \
		txt = open("Cargo.toml").read(); \
		m = re.search(r"^version\s*=\s*\"(\d+)\.(\d+)\.(\d+)\"", txt, re.M); \
		maj, min_, pat = map(int, m.groups()); \
		kind = "$(1)"; \
		ver = f"{maj}.{min_}.{pat+1}" if kind == "patch" else f"{maj}.{min_+1}.0" if kind == "minor" else f"{maj+1}.0.0"; \
		txt = txt[:m.start()] + f"version = \"{ver}\"" + txt[m.end():]; \
		txt = re.sub(r"^(diving-(?:core|app)\s*=\s*\{\s*version\s*=\s*\")(\d+\.\d+\.\d+)(\")", rf"\g<1>{ver}\g<3>", txt, flags=re.M); \
		open("Cargo.toml", "w").write(txt); \
		print(ver)') \
	&& cargo update -p diving-core -p diving-app -p diving -p diving-web --offline >/dev/null \
	&& git add Cargo.toml Cargo.lock \
	&& git commit -m "chore: version $$NEXT" \
	&& git tag "v$$NEXT" \
	&& echo "Bumped to $$NEXT (tag v$$NEXT). Run 'git push && git push --tags' to publish."
endef

.PHONY: version-patch version-minor version-major lint fmt test build-web build-lib build-bin build-web-bin release-web release publish hooks

version-patch:
	$(call bump_version,patch)

version-minor:
	$(call bump_version,minor)

version-major:
	$(call bump_version,major)
