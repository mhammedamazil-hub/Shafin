# VL scene language - quick reference

A VL program describes a 3D scene. The `vl` tool turns it into a photo-like image.
Units: metres and degrees. +Y is up, the floor is at y = 0, and by default the camera
stands on the +Z side looking towards -Z, with +X on its right.

## Statements

```
kind name { property: value, property: value }    an object; `name` is how you refer to it later
let x = 1.5                                       a variable
for i in 0..5 { ... }                             repeat with i = 0, 1, 2, 3, 4
if x > 1 { ... } else { ... }
define chair(height = 0.45) { ... }               a reusable kind; use it as:  chair c1 { at: [1, 0] }
group g { ... objects ... }                       objects that are placed together
```

Comments start with `#` or `//`. Separate properties with commas or new lines.
Objects must be defined before other objects refer to them.

## Shapes

```
sphere   { radius: 0.5 }
box      { size: [width, height, depth], round: 0.01 }     round = edge radius
cylinder { radius: 0.5, height: 1 }                        stands upright
cone     { radius: 0.5, top_radius: 0, height: 1 }
torus    { radius: 0.5, tube: 0.1 }                        lies flat
capsule  { radius: 0.25, height: 1 }
plane    { }                                               the infinite floor; most scenes need one
disk     { radius: 0.5 }       rect { size: [width, depth] }    flat, horizontal
```

## Placing objects

Prefer relations; the tool computes exact contact, so nothing floats or sinks.

```
on: table                 stand on top of `table`;  add  at: [dx, dz]  to shift sideways from its centre
right_of: table           also  left_of  in_front_of  behind  above  below ;  add  gap: 0.1
at: [x, z]                stand on the floor at that spot
at: [x, y, z]             centre of the object in space (only when nothing else fits)
rotate: 30                turn about the vertical axis;  rotate: [rx, ry, rz]  for all three
scale: 2
```

With no placement at all, an object stands on the floor at the origin.
Measurements of an object can be used as numbers:
`table.top  table.bottom  table.center  table.size  table.width  table.height  table.depth`

## Colour and material

```
color: #rrggbb            or a colour name (red, white, navy, ...) or rgb(r, g, b) with values 0..1
material: wood            one of the presets below, or the name of a material you defined
```

Presets: `matte plastic glossy rubber ceramic paper fabric leather glass frosted_glass water
mirror chrome gold silver copper brass steel iron aluminium wood marble concrete stone`

A material of your own, and a procedural texture written with the surface position `P` (metres):

```vl
material oak { preset: wood, color: #8a5a33 }
material tiles { color: mix(#ffffff, #223366, step(0.5, fract(P.x * 5))), roughness: 0.3 }
plane ground { material: tiles }
box shelf { size: [1, 0.04, 0.3], material: oak }
```

Material properties: `preset color roughness metallic specular ior transmission coat emission
emission_strength bump`. Texture functions: `noise(p) fbm(p, octaves) voronoi(p) mix(a, b, t)
smoothstep(a, b, x) step(edge, x) fract floor sin cos abs min max clamp pow length`.

## Light

```
sky { sun_elevation: 40, sun_azimuth: 135 }        daylight; this is the default when nothing is written
environment { color: white, strength: 500 }        even, soft light from all around instead of a sky
light lamp { at: [0, 2, 0], power: 800, temperature: 2700 }                      a bulb; power in lumens
light panel { shape: rect, size: [1, 0.5], at: [0, 2.4, 0], aim: table, power: 3000 }
```

Sun azimuth: 180 = behind the camera (front-lit scene), 90 = from the right, 270 = from the left,
0 = in front of the camera (back-lit).

## Camera and output

```
camera { look_at: table, azimuth: 30, elevation: 20, fov: 35 }
camera { look_at: vase, from: [2, 1.5, 3], f_stop: 2.8, focus: vase }
render { size: [1280, 720], samples: 64 }
```

Without `from`, the camera is placed automatically so that `look_at` fills the frame; `azimuth`
turns it around the target (0 = front, 90 = right side) and `elevation` raises it. Without any
camera, the whole scene is framed. `f_stop` blurs what is out of focus (smaller = more blur).

## Complete example

```vl
plane ground { material: concrete }
box table { size: [1.4, 0.75, 0.8], material: wood }
sphere ball { radius: 0.1, on: table, at: [-0.3, 0], color: red, material: plastic }
cylinder mug { radius: 0.04, height: 0.1, on: table, material: ceramic }
box book { size: [0.2, 0.03, 0.14], on: table, at: [0.35, 0.1], color: navy }
for i in 1..4 {
    # a stack: each book rests on the one before it (inside a loop a name means the latest one)
    box book { size: [0.2, 0.03, 0.14], on: book, rotate: i * 12, color: hsv(i * 80, 0.6, 0.6) }
}
light lamp { at: [0.5, 1.8, 0.5], power: 1200, temperature: 3000 }
camera { look_at: table, azimuth: 25, elevation: 20 }
```
