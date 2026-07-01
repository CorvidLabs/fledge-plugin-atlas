---
spec: threemd.spec.md
---

## Test Plan

### Unit Tests

- Markdown XSS allow-list: `inline("<img src=x onerror=alert(1)>")` escapes to
  `&lt;img ...&gt;`, producing no live tag.
- Link scheme allow-list: `[x](javascript:alert(1))` renders `href="#"`;
  `[x](https://a.b)`, `[x](http://a.b)`, `[x](mailto:a@b)`, and relative
  `[x](../c.md)` keep their href.
- Href quote encoding: a URL containing `"` renders with `%22` in the href.
- Cross-link rendering: `[[z=2|see 2]]` becomes `<a class="xlink" data-z="2">`.
- `mdToHtml` structure: headings map `#`..`######` to `<h1>`..`<h6>`; runs of
  `-`/`*` lines produce a single `<ul>` with `<li>` items; blank lines close the
  list; other lines become `<p>`.
- Label formatting: a plane with `z: '-'` omits the `z=` prefix; a plane with
  `z: 2` yields `z=2 - <label>  (i/n)`.
- `show(n)` clamping: `show(-1)` stays at 0 and `show(n)` past the end stays at
  the last plane.
- `commas(1234567)` returns `1,234,567`; `stubSpec` uses "1 file" vs "2 files".

### Integration Tests

- Stub equals `--scaffold`: render an atlas for a fixture project, click
  `copy-stub`, and assert the copied text is byte-for-byte identical to
  `fledge-atlas --scaffold` output for `clusters[0]` (frontmatter, headings,
  file list, LOC grouping, singular/plural).
- Copy fallback: with `navigator.clipboard.writeText` stubbed to reject, assert
  a textarea is created, `document.execCommand('copy')` is invoked, the flash
  fires, and the textarea is removed; with both paths failing, assert
  "copy failed" flashes.
- End-to-end viewer: load a page with a `.tmd` deck, verify plane 0 shows on
  init, prev/next/slider move and clamp, and clicking a rendered `.xlink` jumps
  to the matching plane.
- CTA payloads: `copy-json` copies `#atlas-data` text; `copy-review` and
  `copy-orphans` produce the expected rows (orphans sorted by LOC desc) and
  flash the count; `copy-stub` with empty `clusters` flashes "no orphan
  cluster".
- Graceful degradation: missing `#atlas-data` and invalid JSON both leave the
  page inert with no thrown error.
