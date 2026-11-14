# Architecture

This document describes what is implemented, why it is built this way, and every
approximation the renderer makes on purpose. It is kept short and factual; numbers
quoted here come from tests in the repository unless stated otherwise.

## The idea

A text-only language model cannot emit pixels at 4K (see `docs/research/00-source-notes.md`:
direct pixel, codec or token emission needs image-trained weights and tens of thousands of
tokens even for tiny images). What such a model *can* do is write a compact program. So:

```
scene description (text)
   -> VL program           written by the model: named objects, relations, materials, lights
   -> validated scene IR   compiler: exact placement, diagnostics with concrete fixes
   -> inspection report    renderer buffers, summarised in words and numbers (the model's "eyes")
   -> image                physically based path tracer, deterministic, any resolution
```

The model never sees an image. It gets two kinds of feedback, both text: compiler
diagnostics, and a measured description of the picture (`vl inspect`).

This is not a new idea (Scene Language, SceneCraft, 3D-GPT, LL3M and POV-Ray's scene
language are prior art). What this project adds is a self-contained runtime with no
Blender or asset library, feedback that needs no vision model, and bit-reproducible output.

## Crates

All code is Rust with **zero third-party crates** and `#![forbid(unsafe_code)]` everywhere.
The development sandbox cannot reach crates.io, so this was forced; it also means offline
builds, no supply chain, and that every numeric routine is in-tree and can be made
bit-reproducible.

| Crate | Role |
|---|---|
| `vl-core` | deterministic math (`dmath`), vectors/matrices, hashing + PCG32, JSON, colour, diagnostics, ordered thread pool |
| `vl-image` | DEFLATE/zlib, PNG decode/encode, linear-light RGBA raster type |
| `vl-ir` | backend-neutral scene, shading-expression VM, noise, physical sky |
| `vl-lang` | VL lexer, parser, evaluator, material presets, lowering to the IR |
| `vl-render` | BVH, shape intersection, BSDF, path tracer, exposure/tone mapping, inspection |
| `vl-cli` | the `vl` command |

`vl-ir` is the contract: nothing in it knows about VL syntax or about any renderer, so
another backend (for example an exporter to a reference renderer) can consume the same scene.

## Reproducibility

* Transcendental functions go through `vl_core::dmath`, built only from IEEE-exact
  operations (`+ - * /`, `sqrt`), because platform `libm` results differ.
* Every sample's random stream is derived from (seed, pixel, sample index) with integer
  hashing, so the result does not depend on thread count or tile order.
* Tested: 1 thread and 4 threads give bit-identical frames; rendering a region reproduces
  exactly the pixels of the full frame (`tests/physics.rs`). PNG output is identical for any
  thread count (`vl-image`).
* **Not yet tested:** identical output across operating systems or CPU architectures. The
  design intends it, and noise has pinned bit patterns, but no cross-platform run has been
  compared. Treat cross-platform bit-identity as a goal, not a result.

## The language (VL)

Reference: `skills/vl-scene/references/language.md` (one page; tests compile every example
in it and check that every preset, function and property it lists exists).

Design choices, each motivated by a documented weakness of language models:

* **Relational placement** (`on:`, `right_of:`, ...) with exact contact computed from
  bounding boxes. Models that emit absolute coordinates produce out-of-bounds placements and
  collisions; here an object with no placement stands on the floor rather than half-buried.
* **Every object is named**; diagnostics, inspection and (later) edits refer to names.
* **Physical units** - metres, lumens, kelvin, exposure values - because that is how a
  model's world knowledge is expressed.
* **One expression language** for numbers and for per-pixel procedural textures. An
  expression that touches the surface position `P` becomes a small shading program;
  constants are folded by running the same VM the renderer runs.
* **Diagnostics name the fix**: stable code, location, "did you mean", the valid properties.
  The parser recovers and reports all errors in one pass. Semantic warnings (floating,
  sunk, overlapping) are held back while errors exist, so they never describe side effects
  of a typo.

Placement is sequential (definition order), not a constraint solver. Bounding boxes are
axis-aligned, so contact for rotated or round objects is the contact of their boxes.

## The renderer

Unidirectional path tracing with next-event estimation and multiple importance sampling.
One principled BSDF (Lambert diffuse; GGX specular with Schlick Fresnel; GGX reflection and
refraction with exact dielectric Fresnel; GGX clear coat), sampled with the distribution of
visible normals. Thin-lens camera. Radiance is in cd/m2, so a sunlit scene needs about EV 15.

### What is validated (`crates/vl-render/tests/physics.rs`, `src/bsdf.rs`, `src/geom.rs`)

| Check | Result |
|---|---|
| White furnace: matte, clear glass, perfect mirror | mean radiance 0.99998 - 1.00000, no pixel above 1 |
| Sphere light over a matte plane vs closed form | 97.2186 vs 97.2222; 4.20864 vs 4.20867 at 4 m off axis |
| Rectangular light vs first-principles quadrature and the closed-form view factor | within 0.4% (statistical) |
| Disk light on axis vs closed form | 131.89 vs 131.96 |
| Sun + sky on a matte plane vs the sky model's own quadrature | 16102.98 vs 16103.98 |
| Light through a glass pane | 0.9215 vs 0.9216 (two Fresnel interfaces) |
| BSDF sampling vs deterministic quadrature, 11 materials x 3 angles | albedo within 0.001; never above 1 |
| Ray intersection, 8 shapes, vs dense sampling + bisection | error below 1e-4 m; normals match the implicit gradient |

These tests found and fixed real bugs (wrong box face normals, lights shadowing
themselves, mis-weighted microfacet samples). They validate light transport, **not**
photographic realism, and they compare the renderer with analytic results and with its own
sky model, not with an independent renderer.

### Deliberate approximations

* **No refractive caustics.** Light reaching a surface through glass uses straight,
  Fresnel-attenuated shadow rays ("transparent shadows"). A glass ball casts a pale shadow
  without the bright focused spot a real one produces.
* **Mirror-path caustics are dropped** unless `render { caustics: true }`.
* **Path regularisation.** After a diffuse bounce, glossy surfaces met later are treated as
  rougher (GGX alpha at least 0.25). Measured on one development scene at 64 samples: raising
  the floor from 0.09 to 0.25 cut speckle pixels from 701 to 64, and the mean display
  luminance moved by 0.6% (0.1655 to 0.1644; part of that is the removed speckles
  themselves). Reflections seen indirectly are blurrier than reality.
* **Indirect clamping** at 4x the exposure white point, as a safety net. With the floor
  above in place it changed the mean display luminance of the same scene by 0.006% (58
  speckle pixels instead of 64). One scene is not a characterisation; scenes with small,
  very bright lights seen through several bounces will lose more.
* **Approximate sky.** Rayleigh and Mie single scattering are integrated directly; higher
  orders use Hillaire's (2020) isotropic approximation; ozone absorbs. Measured at 60 degrees
  sun elevation, turbidity 3: sun 107.6 klx (clear-sky measurements are around 100 klx),
  sky dome 10.4 klx (measured skies: 10-20 klx, so this is at the low end), zenith 2400
  cd/m2, horizon 9100-9800 cd/m2. Before multiple scattering and ozone were added the dome
  gave 6.7 klx and the horizon rendered dark and greenish. The model has not been compared
  with a reference implementation or with sky measurements beyond these ranges; there are
  no clouds, and the sun below the horizon gives twilight only.
* **Rough metals lose energy** (single-scattering microfacet model: albedo 0.82 at
  roughness 0.6 for a white metal).
* The sky dome is found by BSDF sampling only; the sun and emissive spheres, rectangles and
  disks are sampled directly. Other emissive shapes are found by chance and are noisy (the
  renderer reports this as a warning).

### Not implemented (and reported, never silently ignored)

Fog and other participating media; bloom, vignette, grain; denoising; subsurface scattering;
triangle-mesh import; constructive solid geometry; texture images. `Scene::fog` and the
post-processing fields exist in the IR; the renderer emits a warning when they are set.

## Inspection (`vl inspect`)

Computed at 256 px on the longest side from the object-ID buffer and a 24-sample render:
share of the image per object, how much of each object's own silhouette is visible and what
hides it, position in the frame, apparent colour in plain words, exposure statistics, a text
layout map, and findings (`V101` outside the picture, `V102`/`V103` hidden, `V104` tiny,
`V105` cut off, `V110` mostly empty, `V111` almost black, `V112` blown out).

It measures geometry and exposure. It cannot judge whether a picture is beautiful, whether a
material looks right, or whether the scene matches the request in spirit.

## Limits of the approach itself

* Scenes are combinations of simple solids. There are no people, animals, plants or organic
  shapes, and the language model is the only source of structure.
* Realism depends on procedural materials written in advance (25 presets). Anything outside
  them is as good as the texture expression the model writes.
* Rendering cost: about 6 M rays/s on 2 cores for plain materials; the first example scene
  with procedural wood and concrete took 30 s at 640x360 and 64 samples. 4K has not been
  rendered or timed yet.
