# Changelog

## [v0.1.5] - 2026-07-01

### Other

- Add: integration tests for the engine pipeline (line coverage ~9% -> ~50%) (9909f66)

## [v0.1.4] - 2026-07-01

### Other

- Fix: quadrant label clip and coverage-mode treemap legend (3c6d396)

## [v0.1.3] - 2026-07-01

### Other

- Add: test suite, spec-sync specs + companions, and CI governance gate (a259299)

## [v0.1.2] - 2026-07-01

### Other

- Docs: add live trust-panel demo (augur + attest) (cef0e6a)
- Docs: dogfood spec-sync, add self-atlas demo, refresh README (09a69e0)

## [v0.1.1] - 2026-07-01

### Other

- Update: drop zero-LOC languages, bump 'no spec' gray for contrast (4c0308b)
- Fix: undefined --danger token; align graph with governance key (4ef7042)
- Update: colour treemap/sunburst by governance state, one clean legend (59319aa)
- Update: readable treemap labels + colour legend for treemap and sunburst (280add4)
- Update: colour treemap and sunburst by spec ownership (e6a6631)
- Fix: eliminate content overflow across components and widths (2e4b2fc)

## [v0.1.0] - 2026-07-01

### Other

- Chore: stop tracking .claude worktree/session dir (committed by mistake in merge) (cbe804e)
- Add: agent action plan (--json) and risk hotspots ranking (0dd0d72)
- Add: zero-spec experience upgrades (orphan clusters, stub scaffold, language strip) (acd5fe3)
- Add: keyboard and screen-reader accessibility to the spec/code graph (e58d6ab)
- Add: spec dependency graph (depends_on DAG) (bb811ee)
- Add: inline spec-prose reader on spec cards (272812b)
- Fix: agent-surface correctness + generated-file/output-path safety (5537184)
- Fix: zero-code/low-coverage correctness in verdict, vitals, bar, health (43fa71a)
- Fix: exclude vendored/generated trees + minified bundles from source (e2b4381)
- Fix: escape model strings in tooltips + allow-list 3md link schemes (XSS) (4108741)
- Update: document vitals, spec-debt, delta, delight views, and trust panel (7f80b64)
- Add: optional trust panel sourced from augur and attest (efce19c)
- Add: three SVG delight visuals (treemap, sunburst, churn/coverage) (3467d55)
- Add: --timeline 3md export (one plane per active git week) (c9baa80)
- Add: project vitals cockpit, spec-debt scoreboard, and since-you-last-looked sections (8ab4c38)
- Add: --owns, --since, and --gaps agent flags (81e50d8)
- Update: adopt CorvidLabs kit grammar for section rhythm (bc3ec05)
- Add: inline 3md viewer + call-to-action bar (62aeec9)
- Update: all colour scales now resolve through brand tokens (64696f7)
- Add: --3md spec-deck export (Markdown with a Z axis) (9117eb8)
- Update: pet now uses the official CorvidLabs mark (82abe93)
- Add: show/hide component bar + gamified Corvid Pet (bd814b4)
- Add: bubble/containment graph, agent query flags, brand design-system (257276c)
- Add: interactive graph explorer + contribution calendar (db2ce67)
- Fix: graph.js TDZ crash — declare ageColor before specNodes uses it (9b0c738)
- Add: git-driven spec activity heat map + companions (321ef9e)
- Update: verdict-first redesign for humans and agents (967f988)
- Update: rename 'under spec' stat to 'spec-covered' for clarity (2470f83)
- Fix: clippy lints (strip_prefix, sort_by_key) (7b016fd)
- Add: fledge atlas — spec/code overlap graph with JSON model and coverage overlay (2924095)

