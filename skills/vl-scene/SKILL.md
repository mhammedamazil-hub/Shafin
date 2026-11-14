---
name: vl-scene
description: Create and edit photo-like images by writing VL, a small text language for 3D scenes that the bundled `vl` command renders with a physically based path tracer. Use this when asked to generate, draw, render or modify an image and you can only produce text. The tool reports mistakes with exact fixes and describes the resulting picture in words and numbers, so no image input is needed.
license: MIT OR Apache-2.0
metadata:
  version: "0.1.0"
---

# Making images with VL

You write a scene as a VL program; `vl` checks it, tells you in words and numbers what the
picture will look like, and renders it. You never need to see the image.

## Workflow

1. Read `references/language.md` (one page) before writing anything.
2. Write the program to a file, for example `scene.vl`.
3. Run `vl check scene.vl`. Fix every error it reports - each one names the line and the fix -
   and run it again until it prints `ok`. While errors remain, only errors are shown.
4. Read the warnings. A warning that says something "was read as" or "was used instead" means
   the tool guessed what you meant; if the guess is wrong, write what the hint says.
5. Run `vl inspect scene.vl`. Read the object list, the layout map and the `visual findings`.
   Typical findings: an object is outside the picture, hidden behind another, tiny, or cut off
   by the frame; the picture is too dark. Change the program and inspect again.
6. Run `vl render scene.vl -o image.png`. Add `--width 3840` for 4K. Rendering takes from
   seconds to many minutes depending on size and `samples`.

Add `--json` to any command to get the same information as one JSON object.

## Rules that prevent most mistakes

- Give every object a name and place it with a relation (`on: table`, `right_of: lamp`,
  `in: bowl`) rather than by computing coordinates. Contact is then exact.
- Add a floor (`plane ground { }`) unless the scene is in empty space.
- Use real sizes in metres: a table is about 0.75 high, a mug about 0.1, a room about 2.5.
- Let the camera frame the subject: `camera { look_at: thing }` without `from`. The thing is
  shown with its surroundings; add `distance:` for a close-up.
- Light: with no `light` and no `sky` the scene is in daylight. As soon as there is a `light`
  and no `sky`, it is night and only the lamps shine. `sky { time: sunset }` sets the hour.
- Warnings that say an object floats or overlaps are advice; keep the object as it is when that
  is what you intend (a hanging lamp, a wheel set into a car body).

## Known limits (be honest with the user about them)

- Shapes are simple solids, hollow vessels and combinations of them; there are no people,
  animals, plants, fabric folds or detailed meshes. Pictures look like clean product renders
  of simple objects, not like photographs of the real world.
- Light bent through glass does not form bright focused spots (no caustics), so the surface
  under a glass looks darker than it would in a photograph.
- There is no denoiser: with few `samples`, shadowed areas lit by lamps or by reflected light
  look grainy. Raise `samples` (64 to 256) for a final picture.
- Fog, bloom, vignette and film grain are not available. Images cannot be edited after
  rendering; change the program and render again.
