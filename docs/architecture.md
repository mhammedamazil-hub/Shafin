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

Placement is sequential, not a constraint solver. Bounding boxes are axis-aligned, so
contact for rotated or round objects is the contact of their boxes - with one exception:

* **Things put in vessels.** `cup` is a hollow truncated cone (mug, glass, bowl, plate, pot;
  `tube` is the same without a floor). `in: bowl`, and `on:` such a vessel, rest a thing on
  the inner floor or against the sloping wall, computed from the vessel's real shape. Balls
  settle as a heap: every time a ball is added, all balls in that vessel are moved downhill
  in small steps as long as the heap as a whole gets lower, so a ball rolls off the one it
  was put on and the ones below make room. This is a relaxation, not a physics simulation:
  no friction, no bouncing, later balls rest on earlier ones and never the reverse.
  `crates/vl-cli/tests/placement.rs` checks the result with the renderer's own rays (no
  ball cuts into anything, every ball touches something).

**Whatever has one sensible reading is accepted**, and a warning says how it was read. This
rule came out of the two experiments (`docs/experiments/`): small models write `material:
green`, `light sky { time: morning }`, `on: table, at: [x, y, z]`, `table { size: ... }`
without the kind, or refer to a table that is created three lines further down. Each of
these stops a program for no good reason. The codes:

| | what was written | how it is read |
|---|---|---|
| W340 | an unknown colour word | a near miss as the colour meant, `dark_`/`light_` + colour as that shade, else grey |
| W341 | an unknown material | a near miss or `dark`/`light` + material as that material, else a plain surface |
| W342 | a colour where a material belongs | as the colour |
| W343 | `at: [x, y, z]` beside a relation | the relation decides the height |
| W344 | a statement on an object that exists | ignored (objects cannot be changed) |
| W345 | an unknown property | a unique near miss as that property, else ignored |
| W346 | a misspelt name of something the writer made | as that thing, if only one is close |
| W347 | the sun described in a `light` | as `sky { ... }` |
| W348 | an unknown time of day | daylight |
| W349 | `in:` something too small | put on top |
| W350 | `next_to`, `beside`, `near` | to the right |
| W351 | `table { size: ... }` without its kind | the kind that its properties imply |

Without a warning: a statement that refers to something created further down is read
after the statement that creates it; a word that is also the name of an object keeps its
meaning where a colour or material is expected (`sphere orange { color: orange }`).
Invented statement forms (`car_wheel at: [...]`, `x { ... } for i in 1..4`) stay errors:
they have no single reading.

While errors exist only errors are shown; a message repeated on many lines is shown once
with the lines listed; at most three warnings of one kind are spelt out.

**Automatic camera.** With `look_at: thing` and no position, the picture takes in the thing
and whatever lies within one thing-size of it on every side (a little less downwards), at
the nearest distance from which that box is wholly in view. With no `look_at`, or a point,
the whole scene is framed the same way.

**What lights the scene.** Nothing written and no `light`: daylight. A `light` and no sky
statement: night (the sky model scaled to 1e-5, a moon) so that the lamps matter. `sky { }`
beside lamps: daylight. `sky { time: sunset }` and similar words set the sun.

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
| Ray intersection, 11 shapes (three of them hollow vessels), vs dense sampling + bisection | error below 1e-4 m; normals match the implicit gradient |

These tests found and fixed real bugs (wrong box face normals, lights shadowing
themselves, mis-weighted microfacet samples). They validate light transport, **not**
photographic realism, and they compare the renderer with analytic results and with its own
sky model, not with an independent renderer.

### Exposure, light choice, surfaces in the same place

* **Automatic exposure meters the light, not the picture.** A 160-pixel pre-pass measures
  the light falling on surfaces (radiance divided by surface colour), takes the better-lit
  half without its top 5%, and sets the exposure so that a white surface there comes out
  white. Metering the picture's average instead rendered snow grey. `exposure_bias` works
  as on a camera: +1 is twice as bright.
* **Which light a shadow ray goes to** is chosen 85% in proportion to power over squared
  distance and 15% uniformly. Without the uniform share, a surface in the shadow of a strong
  lamp and lit only by a weak one was speckled (67 of 3000 pixels on one test scene, then 1).
* **Two surfaces in the same place** (within 2e-5 of each other along the ray): the smaller
  object is seen - a rug on the floor, paper on a desk - and of two the same size the one
  written later. Before, rounding decided pixel by pixel. One case comes first: where clear
  glass stands on something solid, a ray meets the solid surface. Met the other way round,
  the ray left the glass and went on from a hair beyond the table top, inside the table,
  and the bottom of every glass looked into a black box.

### Denoising (`src/denoise.rs`)

An edge-avoiding a-trous wavelet filter (Dammertz, Sewtz, Hanika and Lensch, High
Performance Graphics 2010) guided by the per-pixel variance the integrator measures, with
the edge-stopping functions of the spatial filter of SVGF (Schied et al., High Performance
Graphics 2017: normal weight max(0, n.n)^sigma, luminance difference divided by sigma times
the square root of the 3x3-smoothed variance, the smoothed variance used for the weight
only). Sources: `docs/research/00-source-notes.md`, section F. The temporal part of SVGF
does not apply to a still image and is not used. The surface colour of the first hit is divided out, so
texture is not filtered; five passes with reach 1 to 16 pixels; a neighbour contributes only
if its normal and depth fit the same surface and its brightness differs from the centre by
no more than the centre's noise explains. On by default; `render { denoise: false }` or
`--no-denoise` turns it off.

Measured against converged renders of the same scenes:

| scene | samples | error without | error with |
|---|---|---|---|
| fruit bowl in sunlight, 480x270 (8-bit RMSE against 1024 samples) | 32 | 3.77 | 2.50 |
| lamp at night, 360x240 (8-bit RMSE against 768 samples) | 16 | 4.94 | 2.47 |
| the same two kinds of scene at 180x120, relative squared error (the test in `vl-cli/tests/denoise.rs`) | 16 | 1 | 1/1.8 and 1/3.5 |

Mean brightness changed by less than 0.15% in every case. Six unit tests state what the
filter must and must not do (leave a converged frame alone, keep a shadow edge, keep
texture, not leak light between surfaces, remove a lone spike but keep a steady highlight,
not depend on the thread count); each was confirmed to fail when the part of the filter it
is about is switched off.

What it does not do: the one-pixel outlines of objects, where two surfaces share a pixel,
stay noisy; what is seen in a mirror or through glass has no features of its own, so noisy
reflections are smoothed across the edges of the reflected things; soft shadow edges lose a
little sharpness when the noise is strong.

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

Fog and other participating media; bloom, vignette, grain; subsurface scattering;
triangle-mesh import; constructive solid geometry; texture images. `Scene::fog` and the
post-processing fields exist in the IR; the renderer emits a warning when they are set.

## Inspection (`vl inspect`)

Computed at 256 px on the longest side from the object-ID buffer and a 24-sample render:
share of the image per object, how much of each object's own silhouette is visible and what
hides it, position in the frame, apparent colour in plain words, exposure statistics, a text
layout map, and findings (`V101` outside the picture, `V102`/`V103` hidden, `V104` tiny,
`V105` cut off, `V110` mostly empty, `V111` almost black, `V112` blown out).

A language model acts on every finding, so a finding about something that is not a mistake
costs a round and can talk it out of a good picture. Therefore (tests in
`crates/vl-cli/tests/inspection.rs`):

* findings are about whole things, not about the parts of a group (a table top hiding two
  of its own legs is not reported);
* what is seen through glass or water counts as visible;
* things stacked, piled, put inside one another or built into one another (a wheel set into
  a car body) hide each other by design and are not reported as hidden - only what is
  hidden by something that merely stands in front of it;
* what other things stand on may run out of the picture;
* a lamp outside the picture is not reported;
* "tiny" is judged by the size a thing would have unobscured.

It measures geometry and exposure. It cannot judge whether a picture is beautiful, whether a
material looks right, or whether the scene matches the request in spirit. In the second
experiment it reported nothing about four oranges standing in a tower.

## Limits of the approach itself

* Scenes are combinations of simple solids and hollow vessels. There are no people, animals,
  plants or organic shapes, and the language model is the only source of structure.
* Realism depends on procedural materials written in advance (33 presets). Anything outside
  them is as good as the texture expression the model writes. The wood preset still reads
  as ripples rather than grain on a large table top.
* Rendering cost: about 6 M rays/s on 2 cores for plain materials. 4K timing is in
  `docs/STATUS.md`.
