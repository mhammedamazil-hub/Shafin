# Shafin - VL: images from text-only language models

VL is a small text language for 3D scenes, plus a self-contained physically based renderer.
A language model that can only write text writes a VL program; the `vl` tool checks it,
describes the resulting picture in words and numbers (so no vision is needed), and renders it.

```
plane ground { material: concrete }
box table { size: [1.4, 0.75, 0.8], material: wood }
sphere ball { radius: 0.1, on: table, color: red, material: plastic }
camera { look_at: table }
```

**Status: work in progress.** The pipeline runs end to end and the light transport is
validated against closed-form radiometry, but the images are clean computer-graphics
renders, not yet photographs; see `docs/STATUS.md` for exactly what is and is not done.

## Build and run

Rust (stable) is the only requirement; there are no third-party dependencies.

```
cargo build --release
./target/release/vl check   examples/still_life.vl
./target/release/vl inspect examples/still_life.vl
./target/release/vl render  examples/still_life.vl -o still_life.png --width 1280
cargo test --release
```

## Where things are

| Path | Contents |
|---|---|
| `skills/vl-scene/` | Agent Skill: how a model should use the tool, and the one-page language reference |
| `docs/architecture.md` | design, what is validated, every deliberate approximation, known limits |
| `docs/STATUS.md` | current state, open defects, resource usage |
| `docs/research/00-source-notes.md` | literature notes and measured facts about the environment |
| `crates/` | `vl-core`, `vl-image`, `vl-ir`, `vl-lang`, `vl-render`, `vl-cli` |
| `tools/ci/`, `.github/` | workflows that fetch the toolchain and run language-model experiments |

Licence: MIT OR Apache-2.0.
