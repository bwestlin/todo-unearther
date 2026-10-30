<!-- markdownlint-disable MD033 MD041 -->
<div align="center">

# `todo-unearther`

**Find the work buried in your code. Connect it to issues. Report it your way.**

[![CI](https://github.com/bwestlin/todo-unearther/actions/workflows/ci.yml/badge.svg)](https://github.com/bwestlin/todo-unearther/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/todo-unearther.svg)](https://crates.io/crates/todo-unearther)
[![Dependencies](https://deps.rs/repo/github/bwestlin/todo-unearther/status.svg)](https://deps.rs/repo/github/bwestlin/todo-unearther)

![Animated terminal demo](https://raw.githubusercontent.com/bwestlin/todo-unearther/main/assets/todo-unearther-demo.gif)

<sub>Synthetic examples rendered by the real output code. No real repository or account data.</sub>

</div>

`todo-unearther` scans a repository for TODO-style comments and produces a
terminal report, Markdown, JSON, or a standalone HTML document. Optional
read-only GitHub enrichment connects comments to issues; optional Git
attribution separates the person named in a comment from its last editor and
inferred original author.

The default scan is local: no API calls, authentication, Git subprocesses, or
history traversal. Files without marker text bypass language parsing.

## Install

Build and install this checkout with a current stable Rust toolchain and a C/C++
compiler for the bundled Tree-sitter grammars:

```sh
cargo install --path . --locked
```

After the first crates.io release, installation is also available with
`cargo install todo-unearther --locked`. Release-tag builds produce binaries
for Linux, macOS, and Windows through GitHub Actions. The installed executable
does not require Rust, Node.js, or a separate Tree-sitter installation.

## Quick start

```sh
todo-unearther
todo-unearther /path/to/repository
todo-unearther --output TODO.md
todo-unearther --format json | jq '.findings[] | .message'
todo-unearther --format html --output todos.html
todo-unearther --group-by file --detail summary
todo-unearther --group-by component --component-group-by file
todo-unearther --group-by component --component-layout tree
todo-unearther --group-by component --component-layout tree --width 80
todo-unearther --group-by component --open
todo-unearther --markers TODO,FIXME,NOTE --exclude '**/fixtures/**'
todo-unearther --timings
```

Without a path, the containing Git repository is scanned, or the current
directory when outside a repository. Root discovery works without the Git
executable, including worktrees. An explicit directory sets the scan boundary.
Nested repositories and symlinks are not traversed.

## Output

| Format | Behavior |
| --- | --- |
| `console` | Terminal layout, colors, and source/issue hyperlinks |
| `text` | Plain text without terminal escape sequences |
| `markdown` | Linked Markdown, suitable for a committed `TODO.md` |
| `json` | Complete structured report with versioned metadata |
| `html` | Standalone responsive document with light/dark and print styles |

The default is console output on an interactive terminal and text when piped.
`--output` infers the format from `.md`, `.json`, `.html`, or `.txt`; unknown
extensions select Markdown. An explicit `--format` takes precedence. CLI output
paths are relative to the working directory. `--output -` means stdout.

`--detail summary` reduces human-readable output; JSON always stays complete.
`--group-by file|marker|component` controls grouping, defaulting to `file`.
Each TODO appears once, with any issue references beneath it. There are no
issue-first groups or labels for comments without an issue reference.

Console output detects the terminal width; `--width 80` overrides it. Plain text
uses 100 columns by default and accepts the same flag or `report.width` setting.
Headings, long paths, issue titles, and multiline descriptions wrap to this
width. Continuation text aligns with the start of the description. Below 100
columns, or when a location leaves too little room, all descriptions start on
their own indented lines. Very deep tree prefixes are compacted to leave room
for text. Markdown, HTML, and JSON ignore the width setting.

Console output leaves one empty row between every rendered line, including
headers, wrapped comments, issue details, and summaries. Set `--line-spacing 0`
for compact output, or `--line-spacing 2` for more space. The repository setting
is `report.line_spacing = 1`. Tree guides continue through these rows. Spacing
replaces existing separators rather than adding extra gaps; there is no trailing
spacer row. Text, Markdown, HTML, and JSON ignore this console setting.

Set `NO_COLOR=1` to disable colors. `--links disabled` disables terminal
hyperlinks. Markers have distinct background colors in console output; HTML
uses matching badges. Source locations and compact issue labels are clickable
in supporting terminals. Pipes, exports, and `TERM=dumb` never receive terminal escapes.
File exports are replaced atomically. Handwritten files require `--force`;
symlink destinations are always rejected. Parent directories must already exist.
Generated Markdown includes targeted suppressions for long lines (`MD013`),
potentially repeated source headings (`MD024`), and multiline `<br>` tags (`MD033`).

### Browser previews

`--open` renders an HTML report into a uniquely named `todo-unearther-*.html`
file in the OS temporary directory and opens it with the default desktop
application. The path is printed on stderr and the file is retained after the
command exits, including when the browser launcher fails. Source links are
absolute, encoded `file://` URLs so they work from the temporary directory;
normal exports keep relative source links.

`--open` conflicts with explicit `--format`, `--output`, and `--check`. It
overrides configured export format/output settings without modifying those
destinations. Policy violations still produce a preview and return `1`;
strict operational failures prevent creation. This feature requires a desktop
application capable of opening HTML, unlike normal file/stdout exports.

## Component grouping

```sh
todo-unearther --group-by component
todo-unearther --group-by component --component-group-by marker
todo-unearther --group-by component --component-group-by file --output TODO.md
todo-unearther --group-by component --component-layout tree --open
todo-unearther --group-by component --check
```

Component sections contain file groups by default. Select `marker` or `file`
with `--component-group-by`. A component owns every finding under its
nearest containing package directory, regardless of source language; nested
packages take precedence. Component totals count each finding once. JSON keeps flat findings, adds
`component_id`, and includes a component table with identities, manifests,
scan-relative roots, and detection status.

Component headings put the folder first, for example `crates/api — api [Cargo]`.
Components are sorted by folder path, with parents before their descendants,
regardless of package names. The default `--component-layout flat` keeps one
section per component. Choose `--component-layout tree` to show the directory
hierarchy, including intermediate folders:

```text
. — 3 findings
├── crates — 2 findings
│   └── api — api [Cargo] — 2 findings
│       └── src/lib.rs
│             2 findings
│
└── packages — 1 finding
    └── web — @acme/web [npm] — 1 finding
        └── src/app.ts
              1 finding
```

This example uses `--component-group-by file --detail summary`. Each directory
total counts its subtree once; the groups inside a component contain only the
findings owned by that component. Paths are relative to the enclosing component;
links still point to the full source location. The tree layout also works with
marker grouping, full details, Markdown, and HTML previews. Markdown uses nested
lists; HTML uses nested sections. JSON keeps the same flat findings and component
metadata in either layout. Set `report.component_layout = "tree"` to make this
the repository default. Layout settings have no effect outside component mode.

| Ecosystem | Static component detection |
| --- | --- |
| Cargo | `Cargo.toml` with `[package]`, named by `package.name`; virtual workspaces alone are not packages |
| npm/Yarn/pnpm/Bun | `package.json` and its `name`, including scoped/private and nested packages |
| Python | PEP 621 `[project].name`, then Poetry metadata in `pyproject.toml`; legacy `[metadata]` in `setup.cfg`, or a `setup.py` boundary without executing it |
| Go | `go.mod` and its module path; `go.work` is a workspace rather than a component |
| Maven | `pom.xml`, named by the project's own `artifactId`, not its parent or dependencies |
| Gradle | `build.gradle(.kts)` and settings-defined projects, including literal `include`, `rootProject.name`, and `project(...).name`/`projectDir = file(...)` declarations |
| .NET | `.csproj`, `.fsproj`, and `.vbproj`, named by project filename; solutions do not create components |

Files outside a package belong to **Repository / shared**. Co-located manifests
share one directory component and retain all ecosystem identities; duplicate
package names remain distinct by root path. Python naming prefers modern
project metadata over legacy metadata in the same directory. Missing or
computed names fall back to the scan-relative directory, or **Repository root**
for `.`. Inferred and unresolved identities are marked in the report.

Detection is static and best-effort: it never invokes build tools, evaluates
scripts, resolves dependency graphs or MSBuild source globs, follows external
workspace members, or expands the scan into ignored directories, nested
repositories, or symlinks. Gradle supports literal declarations, not arbitrary
Groovy/Kotlin evaluation. Tool-only Python configuration does not create a
package. Manual component overrides are not currently supported.

Component detection runs only in component grouping mode. Manifest facts read
during the normal scan are reused; ancestor directory and metadata lookups are
cached within the run. Source file include/exclude patterns need not include
manifest files for ownership to work. Relevant malformed/unreadable manifests
warn and retain an unresolved directory boundary; `--strict` fails instead.
`--timings` includes component assignment time and cache/read counts.

Component-aware source fingerprints cover ownership and displayed identities,
so package renames and boundary changes invalidate component reports. Existing
report fingerprints and JSON shapes are preserved when component grouping is
disabled. Absolute preview paths are never included in fingerprints.

This support matrix describes package detection, not comment parsing. The
precise comment parsers below are unchanged; Java, Kotlin, C#, F#, and VB source
continue to use the existing heuristic fallback where applicable.

## Comment syntax and languages

The default markers are `TODO`, `FIXME`, `HACK`, `XXX`, and `BUG`, matched
case-insensitively at the beginning of comment content. `--markers` replaces
the list with comma-separated ASCII word markers. An optional `@` is accepted.

```rust
// TODO: validate the request
// TODO(bwestlin): handle expired credentials
// TODO:(bwestlin) handle expired credentials
// TODO: (bwestlin): handle expired credentials
// FIXME(#123): reject malformed payloads
// TODO: account for acme/api#42
//   This indented line continues the description.
/* HACK: replace this workaround
 * when the upstream implementation is available.
 */
```

Parenthesized annotations are preserved as literal labels, not verified
identities. Text after them is the message. GitHub references are extracted from
both annotations and messages.

Bundled comment parsers cover Rust, JavaScript/JSX, TypeScript/TSX, Python, Go,
C, C++, and Bash, including shell shebangs. Markers inside strings and Python
docstrings are not comment findings. Inline comments and documentation comments
are supported. Block comments continue until a blank line or another marker;
adjacent line comments continue at the same or greater content indentation.
Code, a blank comment line, a less-indented comment, or another marker ends the
description. Standard markers still delimit descriptions when a custom marker
list is selected. Issue URLs on continuation lines are recognized too.

Other file types use conservative heuristics: a marker after a leading comment
delimiter, or a standalone `MARKER:` line. JSON identifies these findings as
`heuristic` and retains exact line ranges; human-readable reports omit these
parsing details. Use `--no-fallback` for parser-only results. Heuristics may match
documentation examples and cannot guarantee correct parsing of every language.

## Scan boundaries and speed

Traversal respects `.gitignore` and `.ignore` files from the scan root downward,
plus repository-local excludes; global Git ignore configuration is deliberately
not used. Hidden source/configuration files are included. Dependencies, build
directories, VCS metadata, nested repositories, and report destinations are
excluded by default. Reports bearing the generated-file identifier are skipped.

```sh
todo-unearther --jobs 8 --timings
todo-unearther --include 'src/**' --exclude '**/generated/**'
todo-unearther --no-default-excludes --no-ignore
todo-unearther --max-file-size 4194304
```

The default size limit is 1 MiB. Binary files, oversized files, and marker
candidates with invalid UTF-8 are skipped and counted. File-read and traversal
errors fail the scan instead of silently producing an incomplete report.
The scanner reads files once, prefilters literal markers, and parses candidates
in bounded parallel workers. `--timings` prints stage durations, candidate
counts, and bytes read to stderr. There is no persistent cache.

## GitHub connections

```sh
export GITHUB_TOKEN=github_pat_...
todo-unearther --github
todo-unearther --github --repository acme/api
todo-unearther --github --github-host github.example.com \
  --api-url https://github.example.com/api/v3
```

References can be `#123`, `owner/repo#123`, or a full issue/PR URL. An explicit
repository resolves shorthand; otherwise enrichment inspects the GitHub
`origin` remote. Without enrichment, explicitly qualified references and URLs
are still recognized. Unresolvable shorthand remains visible.

Enrichment adds titles, issue/PR identity, and canonical URLs. Routine reports
omit state, labels, and assignees. State is read and retained only when an
explicit closed-issue policy needs it; it is not displayed beside TODOs.
Issue URLs are shown as compact `owner/repo#123` labels, including the host for
Enterprise references. Standalone issue-link lines move to the reference row;
original comment text and full paths remain intact in JSON. Each unique
reference is requested once per run, with at most four concurrent requests.
Authentication uses `GITHUB_TOKEN`, then optional `gh auth token` fallback;
public repositories also work anonymously. Disable the fallback with
`github.gh_auth = false`. Credentials are used only with the configured API
endpoint; redirects are not followed. Only the configured GitHub host is queried.

Use a token with read access to the relevant private issues and pull requests.
The tool never creates issues, edits comments, or changes repository contents
apart from the explicitly requested report/configuration file. A 404 is reported
as **unavailable**, since it can also mean insufficient access. Authentication,
rate-limit, network, and timeout errors are separate states.

## Git attribution

```sh
todo-unearther --attribution blame
todo-unearther --attribution history --history-limit 100 --git-timeout 10
todo-unearther --attribution history --git-detect-copies true
todo-unearther --attribution blame --git-ignore-whitespace false
```

Attribution is off by default. Blame reports the marker line's last editor,
commit, and author date. History tracing searches for its introduction through
Git's line history. Literal annotations, blame authors, and inferred origin
authors are separate fields; the text in `(bwestlin)` is not treated as proof of
who wrote the comment.

Defaults: ignore whitespace, detect within-file moves, disable cross-file copy
detection, inspect at most 200 line-history records per finding, and allow a
total of 30 seconds of Git work per file. At most two Git processes run at once.
Blame is batched by file, and repeated history lookups are reused within a run.

History follows the committed marker line supplied by blame, including Git's
rename tracking. Origin is explicitly **inferred**, not a universal provenance
guarantee: copies, substantial rewrites, merges, and ambiguous line mappings can
obscure the original author. Shallow history, time/revision limits, untracked
files, and uncommitted marker edits produce incomplete/unavailable results.
The tool never fetches, deepens clones, or traverses unrelated branches.

## CI checks and policies

Generate and commit a report, then compare its source fingerprint in CI:

```sh
todo-unearther --output TODO.md
todo-unearther --check
```

Markdown, JSON, and HTML exports include versioned source-fingerprint metadata.
The fingerprint covers source findings and relevant scan/render settings, not
absolute checkout paths, worker counts, issue metadata, Git attribution, or
timestamps. Source checks ignore manual edits to rendered report contents.

Source freshness needs neither Git history nor network access. Configured
enrichment is skipped unless an issue policy needs it. Offline APIs, inaccessible
issues, and missing Git do not change the source fingerprint.

```sh
todo-unearther --check --compare full
todo-unearther --check --compare full --strict
todo-unearther --max-total 100 --fail-on-closed
```

Full comparison checks the entire rendering, including issue/attribution changes.
When enrichment cannot complete, it warns and skips that comparison while still
running independent policies. `--strict` fails instead. Plain text supports full
comparison only. Old reports without metadata must be regenerated or compared
in full. Missing report files are stale; checks never write files.

Policies are disabled by default. Configure total/per-marker limits, required
references for selected markers, and failures for closed/unavailable references.
Issue policies require `--github` or `github.enabled = true`. An unevaluable
policy warns and is explicitly skipped; `--strict` turns that into an error.
An evaluated violation always fails, including a configured policy against a
404 response. Network/authentication failures are not mistaken for 404s.

| Exit code | Meaning |
| --- | --- |
| `0` | Success; optional enrichment warnings may be present |
| `1` | Stale report or evaluated policy violations |
| `2` | Configuration/source-scan error, or strict enrichment failure |

Normal generation writes its report before returning policy violations.
Strict operational failures preserve the previous output.

Example GitHub Actions step after installing the executable:

```yaml
- name: Check TODO report
  run: todo-unearther --check
```

## Configuration

```sh
todo-unearther config init
todo-unearther --config ./tools/todos.toml
todo-unearther completions bash
```

Configuration initialization creates a documented `.todo-unearther.toml` and
preserves an existing file unless `config init --force` is supplied. CLI options
override configuration; output paths in configuration are relative to the
configuration file. Include/exclude globs remain relative to the scan root.
Unknown keys and invalid values are errors.

```toml
[scan]
markers = ["TODO", "FIXME", "HACK", "NOTE"]
exclude = ["**/fixtures/**"]

[report]
group_by = "file"
component_group_by = "file" # file or marker; used only with group_by = "component".
component_layout = "flat" # flat or tree; used only with component grouping.
# width = 100 # Console detects terminal width; text defaults to 100.
line_spacing = 1 # Empty rows between every console line; 0 is compact.

[components]
ecosystems = ["cargo", "npm", "python", "go", "maven", "gradle", "dotnet"]

[github]
enabled = false
repository = "acme/api"

[git]
attribution = "off"

[check]
compare = "source"
strict = false

[policy]
max_total = 100
max_by_marker = { FIXME = 10 }
require_references = ["FIXME"]
fail_on_closed = false
fail_on_unavailable = false
```

## Development

```sh
just check-all
just package
just bench
just demo
```

The benchmarks generate temporary 10k/100k-file repositories with sparse,
dense, dependency-heavy, and mixed-monorepo cases. They measure scanning/fingerprinting and
rendering separately for console, Markdown, JSON, and HTML. Change sizes and
repeat counts using `TODO_BENCH_FILES=10000 TODO_BENCH_RUNS=1 just bench`.
Fixture creation is excluded from reported durations; generated fixtures begin
in the OS page cache, so these are warm-cache measurements rather than cold-disk
claims. GitHub and history are excluded from the default scan benchmark.
Mixed-monorepo runs compare ordinary file grouping with component grouping
over the same files and report manifest/cache lookup counts. See
[BENCHMARKS.md](BENCHMARKS.md) for measurement caveats and initial results.

`just demo` requires ImageMagick and a monospace font. The example executable
constructs synthetic reports and uses the production renderers; the script
captures their ANSI marker/summary colors in SVG frames and turns those scenes
into the animated GIF without scanning real repositories
or accessing credentials/network/history. Component and browser-preview scenes
use the same report data/renderers; `preview.html` is retained alongside the
synthetic terminal scenes for inspection. No browser is launched by the demo.

See [CONTRIBUTING.md](CONTRIBUTING.md) and [RELEASING.md](RELEASING.md).

## Alternatives and scope

[rusty-todo-md](https://github.com/simone-viozzi/rusty-todo-md) is a good choice
for synchronized Markdown and pre-commit integration.
[ianlewis/todos](https://github.com/ianlewis/todos) offers broad comment scanning
and structured output.
[TODO to Issue](https://github.com/alstr/todo-to-issue-action) provides GitHub
issue synchronization through Actions.

`todo-unearther` emphasizes fast local execution, flexible presentation,
read-only issue connections, optional attribution, and independently configurable
CI checks. Issue creation/synchronization, source rewriting, checklist management,
and natural-language task discovery are outside the current scope.

## License

[MIT](LICENSE).
