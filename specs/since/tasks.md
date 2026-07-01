---
spec: since.spec.md
---

## Tasks

- [x] Locate `#atlas-data` and `#delta-body`; return quietly if either is missing.
- [x] Parse the model JSON from `#atlas-data` textContent inside a try/catch.
- [x] Build the storage key `atlas-lastvisit:<project>` from `document.title` or `model.project`.
- [x] Read and parse the stored last-visit timestamp; treat NaN or a read failure as a first visit.
- [x] Render the `delta-first` note with the tracked spec count on a first visit.
- [x] Filter specs by `updated_ts > last`, sort newest first, and render the `delta-list`.
- [x] Render the `delta-empty` note when nothing changed, including the relative time since last visit.
- [x] Escape all interpolated model values before writing innerHTML.
- [x] Write the current timestamp to localStorage only after the delta is rendered.
- [ ] Add unit and integration coverage per testing.md (first-visit note, delta after change, privacy).
