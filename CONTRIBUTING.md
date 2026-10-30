# Contributing

Use current stable Rust, a C/C++ toolchain, `just`, Node.js for Markdown linting,
and `git-cliff` for changelog checks. Tests use local temporary repositories and
mock HTTP servers; they do not require GitHub credentials or external API calls.

```sh
just check-all
```

Keep extraction, enrichment, policies, and rendering separate. All renderers
consume the shared report model. New extraction behavior should have a realistic
fixture that covers false positives as well as expected findings. Changes to
fingerprint semantics or JSON shapes must account for the metadata schema
version and existing exported reports.

Benchmark performance-sensitive changes in release mode. Keep optional history
and network costs out of the ordinary scan path. Regenerate the synthetic demo
with `just demo` when presentation changes.

Use Conventional Commits for commits and PR titles:

```text
feat(scanner): support an additional comment grammar
fix(check): preserve offline source checks
perf(scanner): reuse parser instances
docs: clarify attribution limits
```

Use `!` or a `BREAKING CHANGE:` footer for incompatible changes. The initial
repository commit predates this policy and is explicitly exempted by SHA in
the changelog configuration. New unconventional commits fail changelog checks.

Do not include real credentials or private source/history in fixtures or demos.
