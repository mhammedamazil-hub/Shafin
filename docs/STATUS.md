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
vl render  scene.vl -o image.png [--width 3840] [--no-denoise]
```

16 300 lines of Rust in six crates (tests included), no third-party dependencies,
`forbid(unsafe_code)` everywhere. 113 tests pass (`cargo test --release`, about two minutes
on two cores).

## Verified by tests

* **Light transport** against closed-form radiometry and first-principles quadrature
  (white furnace; sphere, rectangle and disk lights; sun + sky; glass pane), all within
  statistical error. Table in `docs/architecture.md`.
* **BSDF**: sampling agrees with deterministic quadrature for 11 materials at 3 angles;
  nothing reflects more energy than it receives.
* **Geometry**: 11 shapes (three of them hollow vessels) against a dense-sampling reference
  written independently of the intersection code; normals against the implicit gradient.
* **Placement, checked with the renderer's rays** rather than with the formulas that
  compute it: things put in vessels touch the inside and never pass through it; thirteen
  balls piled in three vessels neither cut into anything nor hover; of two surfaces in the
  same place the same one is seen from every direction.
* **Denoiser**: six stated properties on synthetic frames, and a comparison with converged
  renders of two real scenes.
* **Inspection**: reports real occlusion; stays quiet about parts of groups, piles, things
  behind glass, lamps out of view.
* **Reproducibility on one machine**: identical frames for 1 and 4 threads; a region render
  reproduces the full frame's pixels exactly; the denoiser's result does not depend on the
  thread count; `examples/still_life.vl` at 480 px and 32 samples gives the same PNG, byte
  for byte, with 1 and 2 threads (image hash `f62b9425791e13d0`).
* **Language**: exact contact for relational placement; loops, defines, groups; 33 material
  presets stay in physical range; diagnostics for typical mistakes name the fix; 3 000 random
  programs never panic and a returned scene is always valid. The programs the models wrote
  in the two experiments are kept byte for byte (33 files); tests state for 22 of them
  whether they are accepted or rejected, and why.
* **Documentation**: every example in the skill compiles without warnings; every preset,
  function and property it lists exists.
* **Codecs**: zlib against CPython's; PNG against 26 fixtures from an independent encoder.

A defect found late and worth knowing about: until the last commit of this stage, the
bottom of every glass standing on a table looked into a black hole (the ray left the glass
and continued from just inside the table). Earlier pictures in this repository's history
show it. It is fixed and has a test; I had first written it down as a consequence of the
missing caustics, which was wrong.

For the placement, denoiser and box-face tests the code under test was deliberately broken
in several ways to confirm that the tests fail. Two of seven breakages of the placement
formulas were not caught at first; the test scenes were extended until all were. Two
further breakages only make piles less compact and are not caught, by design of the test.

## Measured, but on one scene or one machine only

* Speed: about 6 M rays/s on 2 vCPUs with plain materials.
* 4K: see "4K render" below.
* Denoiser gain: two scenes (numbers in `docs/architecture.md`).
* Fireflies: one development scene (numbers in `docs/architecture.md`).
* Language-model speeds on a GitHub-hosted runner: `docs/research/00-source-notes.md`, section E.

## 4K render

One run, one scene: `examples/still_life.vl` (12 objects; procedural wood and concrete, glass,
gold, depth of field) at **3840x2160, 64 samples per pixel**, with denoising, on the
development sandbox (2 vCPUs, 3.9 GB RAM, no GPU):

| | |
|---|---|
| Render time including denoising | 997.9 s (16.6 min) |
| Wall-clock time including PNG encoding | 16.7 min |
| Peak resident memory (kernel high-water mark, `tools/dev/measure.sh`) | 874 MB |
| Output | 6.7 MB PNG; decodes as 3840x2160 RGB with an independent decoder (Pillow) |

An earlier run of the same scene without the denoiser took 1125.7 s (18.8 min wall clock)
and 534 MB. The 4K file itself is not in the repository (it is written to the ignored
`out/` folder); `docs/images/still_life_from_4k_1280.png` is a downscale and
`docs/images/still_life_4k_crop_1to1.png` an unscaled 1024x576 crop. At full resolution
the glass and metal are convincing; the image as a whole is a clean computer-graphics
render, not a photograph.

## Language-model experiments

Two experiments, both small: three scene descriptions each, one sample per pair. Full
accounts with every caveat: [experiments/llm-run-1.md](experiments/llm-run-1.md),
[experiments/llm-run-2.md](experiments/llm-run-2.md).

| | experiment 1 | experiment 2 (new descriptions, one new model) |
|---|---|---|
| pairs (model x description) | 9 | 12 |
| program compiles on the first attempt | 2 | 8 |
| program compiles at some point | 3 | 9 |
| first program had errors, later repaired with the tool's feedback | 1 of 5 | 1 of 4 |
| no reply from the hosted service (a fault of the set-up) | 2 | 0 after a retry |
| picture shows the scene asked for (my own judgement, one judge, not blind) | 0 | 1 |
| picture recognisable with a clear fault | not classified | 4 |

The two experiments used different descriptions and the second had a fourth model, so the
columns are not a controlled before-and-after.

What they show together:

* Getting a small model's program to compile is no longer the main obstacle. What 1.5B and
  3B models then build is usually not the scene: they copy the reference's examples, pick
  wrong shapes and sizes, and do not use the tool's feedback (offered the choice, they
  answer "done"; when they do revise, they break as often as they fix).
* The 20B hosted model used the language as intended and produced one correct picture and
  one that was correct except for a fault of the tool (since changed).
* **The claim "a text-only model can make the image it is asked for with this tool" holds,
  on this evidence, for a 20B model on simple scenes, sometimes. It does not hold for 1.5B
  to 3B models.**

After each experiment the language was changed using that experiment's transcripts. The
changes made after experiment 2 have not been tested on new descriptions.

## Not done

* **Photorealism is not achieved.** Images are clean, physically plausible renders of simple
  solids and hollow vessels. Nobody would mistake them for photographs. Missing for that:
  richer geometry (meshes, constructive solid geometry, displacement), camera effects, fog,
  subsurface scattering, more and better materials, clouds.
* **Image editing**: nothing beyond the building blocks (region re-render that reproduces
  pixels exactly; PNG round-trip). No layers, masks, compositing, undo/redo or project format.
  Editing today means changing the program and rendering again.
* **Structured tool interface (MCP)**: not started. The CLI has `--json` output.
* **Research report**: only raw notes exist (`docs/research/00-source-notes.md`), mostly from
  search-result text; three citations are flagged there as unverified.
* **Cross-platform determinism**: designed for, never tested on a second OS or CPU.
* **WebAssembly**: a thread-using smoke binary compiled but panicked under Node; no working
  WASM build exists.
* **Independent renderer comparison** (for example Mitsuba): not done. The renderer is
  validated against analytic results and its own sky model only.
* **Inspection** measures geometry and exposure. It cannot tell whether a material looks
  right or whether the picture matches the request. It said nothing about four oranges
  standing in a tower.

## Known defects

* The wood preset reads as ripples, not grain, on a large table top.
* Glass casts pale shadows without the bright focused spots a real one produces (no
  caustics, by design).
* The denoiser leaves the one-pixel outlines of objects noisy and smooths noisy reflections
  across the edges of what is reflected.
* The sky has no clouds and its dome is at the low end of measured brightness (10.4 klx).
* Rough metals are darker than real ones (no multiple-scattering compensation).
* Fog, bloom, vignette and grain can be requested in the IR but are not rendered (the
  renderer warns).
* Procedural materials are slow: about 4x the cost of plain ones on the example scene.
* Heaps of balls are settled by a relaxation without friction; only balls settle, other
  things stand where they are put or on top of what is there.

## Resources used in the owner's GitHub account

* **Actions runner time** (private repository): about 110 job-minutes in eight workflow
  runs, each job rounded up to a whole minute as GitHub bills them: 1 to build the
  toolchain bundle, 5 in three short probe runs, 29 to measure model speeds, 29 for
  experiment 1, 37 for experiment 2 (four jobs side by side), 9 for the retry of the hosted
  model. GitHub's own timing endpoint reports 0 billable milliseconds for every one of these
  runs, so they may fall within the account's free allowance; the account's billing page,
  which I cannot see, is the authority.
* **Hidden git refs** created by workflows (not branches; they do not show in the UI):
  * `refs/vl-bootstrap/toolchain` - the official Rust toolchain tarballs, about 200 MB.
    Needed only to rebuild the development sandbox. Remove with
    `git push origin :refs/vl-bootstrap/toolchain`.
  * `refs/vl-results/llm-probe` - obsolete (GitHub Models probe). Safe to delete.
  * `refs/vl-results/local-llm-probe` - model speeds.
  * `refs/vl-results/llm-experiment` - experiment 1.
  * `refs/vl-results/llm-experiment-r2-b0` to `-b3`, `refs/vl-results/llm-experiment-r3-b0`
    - experiment 2.
* **Third party**: the hosted backend in the experiments sent the language reference, the
  scene descriptions, the model's own programs and the tool's reports about them to
  `text.pollinations.ai`, an anonymous free endpoint. Nothing else. Remove that backend from
  `.github/vl-requests/llm-experiment.json` to stop this in future runs.

## Next steps, in order

1. Confirm the acceptance criteria with the owner (see the note at the top).
2. Experiment 3 on new descriptions, with the feedback prompt changed so that a model is
   not offered "done" while remarks are open, and a pause of 45 s for the hosted service.
   Costs about 40 job-minutes.
3. Geometry beyond primitives: lathe and extrusion to meshes, constructive solid geometry.
4. Materials: wood, fabric; multiple-scattering compensation for rough metals.
5. Fog; bloom, vignette, grain.
6. Image editing: object-level edits by name with region re-render, then layers and masks.
7. Cross-platform determinism test in CI on three operating systems.
8. Structured tool interface; research report with verified citations.
