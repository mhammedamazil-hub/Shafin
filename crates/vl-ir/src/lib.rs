//! vl-ir: the validated, backend-neutral intermediate representation.
//!
//! The compiler (`vl-lang`) produces a [`scene::Scene`]; rendering backends
//! (`vl-render`, the Mitsuba exporter) and the raster engine (`vl-image`)
//! consume it. Nothing in here knows about VL syntax or about any renderer.
//!
//! * `expr`  - per-shading-point expression programs (procedural textures) and
//!             their reference evaluator, including deterministic noise
//! * `scene` - cameras, lights, materials, geometry instances, media
//! * `sky`   - reference semantics of the physical sky model
#![forbid(unsafe_code)]

pub mod expr;
pub mod noise;
pub mod scene;
pub mod sky;
