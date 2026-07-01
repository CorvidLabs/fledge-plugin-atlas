---
module: since
status: active
version: 0.1.1
owner: CorvidLabs
files:
  - src/since.js
depends_on:
  - engine
---
# since spec

## Purpose

The "Since you last looked" panel: it remembers a reviewer's last visit in the
browser and lists which specs changed since, so a returning reader sees the
delta first.

## Requirements

- Track last-visit state locally in `localStorage`, keyed per project; nothing
  leaves the page.
- On a first visit, record the timestamp and show a neutral "first visit" note
  rather than a misleading empty delta.
- Compare against each spec's `updated_ts` from the model to decide what changed.
