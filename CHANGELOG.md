# Changelog

## Unreleased

## 0.1.1 - 2026-10-30

## 0.1.0 - 2026-10-30

### Added

- Automatic component grouping for Cargo, npm-compatible packages, Python, Go,
  Maven, Gradle, and .NET, with selectable nested grouping and cached detection.
- Folder-first component headings and path ordering, plus optional directory
  trees via `--component-layout tree` or `report.component_layout`.
- Continuous tree branches through spacing between findings and component groups.
- TODO-first reports with file grouping by default, compact subordinate issue
  references, component-relative tree paths, and optional terminal hyperlinks.
- Width-aware console/text wrapping with aligned multiline descriptions,
  `--width` overrides, and colored marker badges in console and HTML.
- Matching marker background colors for summary counts in console and HTML.
- Configurable console line spacing via `--line-spacing` and
  `report.line_spacing`, defaulting to one empty row between all rendered lines
  with continuous tree guides.
- Title/link GitHub enrichment, with issue state retained only for explicit
  policies; removed issue-first grouping and visible parser/line-range notes.
- Same-indent continuation comments, including issue URLs on following lines.
- Targeted markdownlint suppressions in generated Markdown reports.
- `--open` for retained temporary HTML previews with encoded absolute source links.
- Component-aware fingerprints, compatible optional JSON metadata, and mixed-monorepo benchmarks.
- Parallel comment scanning with marker prefiltering and configurable heuristics.
- Console, text, Markdown, JSON, and HTML reports.
- Read-only GitHub issue connections and optional bounded Git attribution.
- Source/full report checks, lenient enrichment handling, and optional policies.
- Synthetic animated demo, benchmarks, and cross-platform release preparation.

### Fixed

- Correct width budgeting for long continuation lines after inline TODO headers.
- Console hyperlinks close at each line boundary and reopen after indentation
  when wrapped, keeping spacer rows and tree guides outside clickable spans.
- Portable canonical-path handling for macOS preview checks, Windows source
  links, and deterministic report generation through directory aliases.

### Documentation

- Use a relative path for the animated demo image in the README.
