# Test fixtures for vcli-perception

All PNG fixtures used by tests in this crate are generated **programmatically**
in the test code — we deliberately do NOT check in binary assets for unit
tests. See:

- `src/template.rs` — `template_png_varied_16x8()` and `template_png_white_16x8()`
  build tiny PNGs at runtime via the `image` crate.
- `src/pixel_diff.rs` — `solid_frame()` builds in-memory RGBA frames.

Integration fixtures (the YT skip-button PNG used in e2e demos) live under
`/assets/fixtures/` at the workspace root, not here.
