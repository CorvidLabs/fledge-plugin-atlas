---
module: depgraph
status: active
version: 0.1.1
owner: CorvidLabs
files:
  - src/depgraph.js
depends_on:
  - engine
---
# depgraph spec

## Purpose

The spec dependency DAG: a directed graph read from each spec's `depends_on:`
frontmatter, with foundational modules settling toward the bottom.

## Requirements

- Size each node by its lines of code; ring the hub nodes many specs depend on;
  draw any dependency cycle in the `--bad` colour.
- Give the SVG viewBox horizontal slack so the labels on edge-most nodes are not
  clipped.
- Fall back to a short explanatory note when no spec declares `depends_on`.
