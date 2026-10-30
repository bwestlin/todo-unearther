default:
    @just --list

fmt-write:
    cargo fmt

fmt-check:
    cargo fmt --check

check:
    cargo check --locked --all-targets --all-features

lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings

test:
    cargo test --locked --all-features

markdown-check:
    npx --yes markdownlint-cli2 '**/*.md' '#target/**' '#CHANGELOG.md'

changelog-preview:
    git-cliff --unreleased

changelog-check:
    git-cliff --unreleased > /dev/null

check-all: fmt-check check lint test markdown-check changelog-check

package:
    cargo package --locked

bench:
    cargo bench --locked --bench reports

demo:
    bash scripts/render-demo.sh

release-dry-run level:
    cargo release {{level}}

release level:
    cargo release {{level}} --execute

_changelog-release version:
    #!/usr/bin/env bash
    set -euo pipefail
    next=$(mktemp ./CHANGELOG.md.XXXXXX)
    trap 'rm -f "$next"' EXIT
    awk -v release="## {{version}} - $(date -u +%F)" \
        '{ print } /^## Unreleased$/ { print "\n" release }' CHANGELOG.md > "$next"
    if [[ "${DRY_RUN:-true}" == "false" ]]; then
        mv "$next" CHANGELOG.md
    else
        cat "$next"
    fi
