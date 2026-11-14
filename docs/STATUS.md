# Status

Last updated: 2026-11-14. This file is the durable record of where the project stands. It
states what was measured, what was only built, and what does not exist yet.

> **Note on requirements.** The owner's original brief is not stored in this repository.
> The work so far follows a staged plan derived from it (research, language, compiler,
> renderer, image editing, skill, experiments, validation, final report). Acceptance criteria
> in the brief's own words should be re-checked against this file before anything is called
> finished. **No acceptance criterion is claimed as met.**

## What runs today

Text in, image out, with no step that needs vision:

```
vl check   scene.vl      compile; errors and warnings with line numbers and concrete fixes
vl inspect scene.vl      measured description of the picture (objects, occlusion, framing, exposure)
vl render  scene.vl -o image.png [--width 3840]
```

13 500 lines of Rust in six crates, no third-party dependencies, `forbid(unsafe_code)`
everywhere. 85 tests pass (`cargo test --release`).

## Verified by tests

* **Light transport** against closed-form radiometry and first-principles quadrature
  (white furnace; sphere, rectangle and disk lights; sun + sky; glass pane), all within
  statistical error. Table in `docs/architecture.md`.
* **BSDF**: sampling agrees with deterministic quadrature for 11 materials at 3 angles;
  nothing reflects more energy than it receives.
* **Geometry**: 8 shapes against a dense-sampling reference; normals against the implicit
  gradient; the box-face test was mutation-checked (it fails when the bug is put back).
* **Reproducibility on one machine**: identical frames for 1 and 4 threads; a region render
  reproduces the full frame's pixels exactly; PNG bytes identical for any thread count.
* **Language**: exact contact for relational placement; loops, defines, groups; 25 material
  presets stay in physical range; diagnostics for typical mistakes name the fix; 3 000 random
  programs never panic and a returned scene is always valid.
* **Documentation**: every example in the skill compiles without warnings; every preset,
  function and property it lists exists.
* **Codecs**: zlib against CPython's; PNG against 26 fixtures from an independent encoder.

## Measured, but on one scene or one machine only

* Speed: about 6 M rays/s on 2 vCPUs with plain materials. The example still life
  (procedural wood and concrete) took 30 s at 640x360, 64 samples.
* 4K: see "4K render" below.
* Fireflies: one development scene (numbers in `docs/architecture.md`).
* Language-model speeds on a GitHub-hosted runner: `docs/research/00-source-notes.md`, section E.

## 4K render

One run, one scene: `examples/still_life.vl` (12 objects; procedural wood and concrete, glass,
gold, depth of field) at **3840x2160, 64 samples per pixel**, on the development sandbox
(2 vCPUs, 3.9 GB RAM, no GPU):

| | |
|---|---|
| Render time | 1125.7 s |
| Wall-clock time including PNG encoding | 18.8 min |
| Peak resident memory (kernel high-water mark, `tools/dev/measure.sh`) | 534 MB |
| Output | 10.6 MB PNG; decodes as 3840x2160 RGB with an independent decoder (Pillow) |

No denoising was applied; a few bright noise specks are visible in the glass at full size.
The 4K file itself is not in the repository (it was written to the ignored `out/` folder);
`docs/images/still_life_from_4k_1280.png` is a downscale of it and
`docs/images/still_life_4k_crop_1to1.png` is an unscaled 1024x576 crop. The wood preset was
improved after this run, so the committed program now renders a somewhat different table.
At full resolution the glass and metal are convincing; the image as a whole is a clean
computer-graphics render, not a photograph.

## Language-model experiment

PENDING_EXPERIMENT

## Not done

* **Photorealism is not achieved.** Images are clean, physically plausible renders of simple
  solids. Nobody would mistake them for photographs. Missing for that: richer geometry
  (meshes, constructive solid geometry, displacement), denoising, camera effects, fog,
  subsurface scattering, more and better materials, clouds.
* **Image editing**: nothing beyond the building blocks (region re-render that reproduces
  pixels exactly; PNG round-trip). No layers, masks, compositing, undo/redo or project format.
* **Structured tool interface (MCP)**: not started. The CLI has `--json` output.
* **Research report**: only raw notes exist (`docs/research/00-source-notes.md`), mostly from
  search-result text; three citations are flagged there as unverified.
* **Cross-platform determinism**: designed for, never tested on a second OS or CPU.
* **WebAssembly**: a thread-using smoke binary compiled but panicked under Node; no working
  WASM build exists.
* **Independent renderer comparison** (for example Mitsuba): not done. The renderer is
  validated against analytic results and its own sky model only.
* **Inspection** measures geometry and exposure. It cannot tell whether a material looks
  right or whether the picture matches the request in spirit.

## Known defects

* The sky has no clouds and its dome is at the low end of measured brightness (10.4 klx).
* Glass casts pale shadows without focused bright spots (no caustics, by design).
* Rough metals are darker than real ones (no multiple-scattering compensation).
* Fog, bloom, vignette and grain can be requested in the IR but are not rendered (the
  renderer warns).
* Procedural materials are slow: about 4x the cost of plain ones on the example scene.

## Resources used in the owner's GitHub account

* **Actions minutes** (private repository, so these count against the account's quota):
  ACTIONS_MINUTES
* **Hidden git refs** created by workflows (not branches; they do not show in the UI):
  * `refs/vl-bootstrap/toolchain` - the official Rust toolchain tarballs, about 200 MB.
    Needed only to rebuild the development sandbox. Remove with
    `git push origin :refs/vl-bootstrap/toolchain`.
  * `refs/vl-results/llm-probe` - obsolete (GitHub Models probe). Safe to delete.
  * `refs/vl-results/local-llm-probe`, `refs/vl-results/llm-experiment` - measurement results.
* **Third party**: the optional hosted backend in the experiment sends the language reference
  and the scene descriptions (nothing else) to `text.pollinations.ai`, an anonymous free
  endpoint. Remove that backend from `.github/vl-requests/llm-experiment.json` to stop this.

## Next steps, in order

1. Read the experiment results; widen it (more models, more prompts) only as far as the
   owner accepts the Actions-minute cost.
2. Denoising (edge-avoiding wavelet filter guided by the albedo/normal/depth buffers the
   renderer already produces).
3. Geometry beyond primitives: lathe and extrusion to meshes, constructive solid geometry.
4. Fog; bloom, vignette, grain.
5. Image editing: object-level edits by name with region re-render, then layers and masks.
6. Cross-platform determinism test in CI on three operating systems.
7. Structured tool interface; research report with verified citations.
