---
module: threemd
version: 1
status: active
files:
  - crates/atlas-core/src/threemd.js

db_tables: []
depends_on:
  - engine
---

# Threemd

## Purpose

The `threemd` module is the client-side JavaScript embedded (via `include_str!`)
into the atlas HTML. It does two things:

1. Inline 3md plane viewer: for each `.tmd` element it lets the reader scrub
   through the ordered planes of a spec deck (prev, next, and a range slider),
   rendering each plane's Markdown body into the stage as safe HTML.
2. Call-to-action buttons: a row of `.btn[data-act]` controls that copy useful
   payloads to the clipboard (the full model JSON, a stub `*.spec.md` skeleton
   for the largest orphan cluster, the verdict text, the review queue, and the
   orphan file list) or jump the page to the 3md deck section.

It exists so a reader can page through a spec's 3md planes without leaving the
atlas, and so both a human and an agent can lift copy/paste-ready starting
material (especially the stub spec) straight out of the rendered page.

## Public API

This module exposes no functions to other scripts. Its contract is the DOM it
binds to and the shape of the model JSON it reads. It runs once as an IIFE on
load.

### DOM contract

| Selector / ID | Role |
|---------------|------|
| `#atlas-data` | `<script>`-style element whose `textContent` is the model JSON. Absent or invalid JSON aborts silently. |
| `.tmd` | A plane viewer container. `data-doc` indexes into `model.threemd`. |
| `.tmd .tmd-plane` | Stage element; receives rendered plane HTML via `innerHTML`. |
| `.tmd .tmd-label` | Text caption; shows `z=<z> - <label>  (i/n)`. |
| `.tmd .tmd-slider` | `<input type=range>`; `value` is the 0-based plane index. |
| `.tmd .tmd-prev` / `.tmd .tmd-next` | Step one plane back / forward. |
| `.xlink[data-z]` | In-plane cross-link (rendered from `[[z=N|label]]`); click jumps to the plane whose `z` matches `data-z`. |
| `.btn[data-act]` | CTA button; `data-act` selects the action. |
| `#act-note` | Flash element; gets `.show` toggled with a status message. |
| `#c-3md`, `.cbtoggle[data-target=c-3md]` | Target section and its expand chip for the `go-3md` action. |

### Model contract (from `engine`)

| Path | Used by |
|------|---------|
| `model.threemd[docIndex].planes[]` | Viewer. Each plane has `md` (Markdown), `z` (level or `'-'`), `label`. |
| `model.clusters[0]` | `copy-stub`. Fields: `module`, `dir`, `files[].path`, `file_count`, `loc`. |
| `model.verdict` | `copy-verdict`. |
| `model.specs[]` | `copy-review`. Filtered on `needs_review`; uses `module`, `review_reason`, `path`. |
| `model.files[]` | `copy-orphans`. Filtered on `orphan`; uses `path`, `loc`. |

### `data-act` actions

| Action | Effect |
|--------|--------|
| `copy-json` | Copies raw `#atlas-data` text; flashes "model JSON copied". |
| `copy-stub` | Copies the stub spec for `clusters[0]`; flashes "<module>.spec.md copied". |
| `copy-verdict` | Copies `model.verdict`; flashes "verdict copied". |
| `copy-review` | Copies one `- <module>: <reason> (<path>)` line per review-flagged spec. |
| `copy-orphans` | Copies orphan file paths (sorted by LOC desc) as `<path> (<loc> LOC)`. |
| `go-3md` | Expands the `c-3md` section if collapsed and smooth-scrolls to it. |

### Safe Markdown renderer

`mdToHtml(md)` walks the body line by line and emits an allow-listed subset:
ATX headings `#`..`######`, unordered list items (`-`/`*`), paragraphs. Inline
handling (`inline`) escapes `&<>` first, then renders `` `code` ``,
`**strong**`, `[[z=N|label]]` cross-links, and `[text](url)` links.

## Invariants

1. The renderer emits only minimal SAFE Markdown. Every line body is passed
   through `esc` (escapes `&`, `<`, `>`) before any inline transform, so a plane
   body cannot inject raw HTML or a `<script>` tag.
2. Link `href` values are scheme-checked against an allow-list: a URL with no
   scheme (relative) or a `https`, `http`, or `mailto` scheme is kept; any other
   scheme (for example `javascript:`) is replaced with `#`. Double quotes in the
   href are percent-encoded. Links open with `target="_blank" rel="noopener"`.
3. The "Copy stub spec" button produces the EXACT `*.spec.md` skeleton that the
   `--scaffold` CLI flag prints for the same cluster, so a human and an agent
   both start the first spec from an identical file: same frontmatter keys
   (`module`, `status: draft`, `version: 0.1.0`, `owner: TODO`, `files:`), same
   headings (exactly `# <module> spec`, `## Purpose`, and
   `## Requirements`), same TODO
   prose including file count, singular/plural, and comma-grouped LOC.
4. Clipboard copy is resilient: it tries `navigator.clipboard.writeText` first,
   and on any failure falls back to a hidden `<textarea>` plus
   `document.execCommand('copy')`; if even that throws it flashes "copy failed".
   The temporary textarea is always removed.
5. Cross-links resolve within a deck: clicking an `.xlink` shows the plane whose
   `z` equals the link's `data-z`; if no plane matches, nothing happens.
6. `show(n)` clamps the index to `[0, planes.length-1]`, so prev at the first
   plane and next at the last are no-ops rather than errors.

## Behavioral Examples

```
Given the atlas has at least one orphan cluster in model.clusters
When the reader clicks the button with data-act="copy-stub"
Then the clipboard holds the stub spec for clusters[0], byte-for-byte
     identical to `fledge-atlas --scaffold` output for that cluster,
     and #act-note flashes "<module>.spec.md copied"
```

```
Given a plane body contains the cross-link [[z=2|see plane 2]]
When the reader clicks the rendered "see plane 2" link
Then the viewer calls show() on the plane whose z === "2"
     and the label updates to that plane's z, label, and position
```

```
Given a plane body contains [click me](javascript:alert(1))
When that plane is rendered into the stage
Then the anchor's href is "#" (the javascript scheme is not allow-listed)
     so the script never executes
```

## Error Cases

| Error | When | Behavior |
|-------|------|----------|
| No data element | `#atlas-data` is absent | IIFE returns immediately; nothing binds. |
| Bad JSON | `#atlas-data` text fails `JSON.parse` | Caught; `return`, no viewer or buttons wired. |
| Missing deck | `.tmd` `data-doc` has no matching `model.threemd` entry | That viewer is skipped (`return` in the loop). |
| Empty deck | A deck has zero planes | Viewer skipped; slider/stage left untouched. |
| No orphan cluster | `copy-stub` clicked but `model.clusters` is empty | Flashes "no orphan cluster"; nothing copied. |
| Clipboard blocked | `navigator.clipboard.writeText` rejects/throws | Falls back to hidden textarea + `execCommand('copy')`. |
| Fallback blocked | `execCommand('copy')` also throws | Flashes "copy failed"; textarea still removed. |
| Missing flash target | `#act-note` is absent | `flash` is a no-op; actions still run. |
| Dangling cross-link | `.xlink` `data-z` matches no plane | Click ignored; current plane stays shown. |

## Dependencies

- `engine`: supplies the model JSON embedded in `#atlas-data`, including the
  `threemd` planes, `clusters`, `verdict`, `specs`, and `files` this module
  reads. The stub-spec skeleton must stay in lockstep with the engine's
  `--scaffold` output.
- Browser platform: the Clipboard API (`navigator.clipboard`) with a legacy
  `document.execCommand('copy')` fallback, and `Element.scrollIntoView`.
- No external libraries, fonts, scripts, or network calls (self-contained).

## Change Log

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07-01 | Initial spec |
