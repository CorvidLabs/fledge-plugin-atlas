---
module: components
status: active
version: 0.1.1
owner: CorvidLabs
files:
  - src/components.js
depends_on:
  - engine
---
# components spec

## Purpose

The show/hide component bar: the sticky toolbar that toggles each atlas section
on or off and remembers the reader's choices.

## Requirements

- Toggle each section's visibility from the sticky bar and persist the hidden
  set in `localStorage`, keyed per project, so choices survive a reload.
- Keep the toggle buttons' `aria-pressed` state in sync with what is shown, for
  assistive tech.
