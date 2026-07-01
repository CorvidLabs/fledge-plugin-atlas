---
spec: threemd.spec.md
---

## Tasks

- [x] Parse `#atlas-data` JSON; abort silently on missing element or parse error.
- [x] Implement `esc` (escape `&<>`) and `inline` (code, strong, cross-links, links).
- [x] Enforce the link scheme allow-list (relative, https, http, mailto); others become `#`; percent-encode quotes.
- [x] Implement `mdToHtml` line walker (headings, `-`/`*` lists, paragraphs) with list open/close handling.
- [x] Bind each `.tmd` viewer: `show(n)` with clamping, prev/next/slider, and label formatting.
- [x] Wire `.xlink` clicks to jump to the plane whose `z` matches `data-z`.
- [x] Implement `flash` (`#act-note` `.show` toggle) and `commas` LOC grouping.
- [x] Implement `stubSpec(c)` matching the `--scaffold` skeleton exactly.
- [x] Implement `copy(text, msg)` with clipboard API + textarea/`execCommand` fallback.
- [x] Wire `.btn[data-act]` handlers: copy-json, copy-stub, copy-verdict, copy-review, copy-orphans, go-3md.
- [ ] Add tests: Markdown XSS allow-list, stub == `--scaffold`, copy fallback (see testing.md).
