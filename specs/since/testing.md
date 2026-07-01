---
spec: since.spec.md
---

## Test Plan

Tests drive the panel in a DOM environment with `#atlas-data` (model JSON) and `#delta-body` present, using a controllable or stubbed `localStorage`.

### Unit Tests

- First visit note: with no `atlas-lastvisit:<project>` entry and a model of N specs, `#delta-body` gets a `delta-first` note stating N specs are tracked, and the key is written to the current Unix second.
- Singular vs plural: a first visit with exactly one spec reads "1 spec ... is being tracked" wording, and with zero specs reads "0 specs".
- Changed set: with a stored timestamp T, only specs whose `updated_ts` is strictly greater than T appear, and they are ordered newest `updated_ts` first.
- Empty delta: with a stored timestamp newer than every `updated_ts`, `#delta-body` gets a `delta-empty` note including the relative time since T.
- Relative time thresholds: `rel` renders `m`, `h`, `d`, `mo`, `y` at the boundaries used in `src/since.js`.
- Meta rendering: a changed spec shows its `module`, and includes `updated` and `commits` meta only when those fields are present.
- Escaping: model values containing `&`, `<`, or `>` are HTML-escaped in the rendered output.
- Guard clauses: a missing `#atlas-data` or `#delta-body`, and an unparseable model JSON, each leave the DOM and storage untouched.
- Corrupt stored value: a non-numeric `atlas-lastvisit:<project>` is treated as a first visit.

### Integration Tests

- First-visit then return: run the panel once against a fresh store (asserts the `delta-first` note and that the stamp is written), advance one spec's `updated_ts` past the stored stamp, run again, and assert that exact spec is listed in the `delta-list`.
- Diff before stamp: confirm the second run diffs against the stamp written by the first run, not against its own current timestamp (an unchanged model on the second run yields `delta-empty`, not a self-diff).
- Privacy: run the full panel with network and any storage sinks observed, and assert only `localStorage` under `atlas-lastvisit:<project>` is touched, with no network request and nothing written outside that key.
