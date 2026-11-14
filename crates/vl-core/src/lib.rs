//! vl-core: dependency-free foundations shared by every other crate.
//!
//! * `dmath`  - deterministic transcendental functions (bit-identical on every IEEE-754 platform)
//! * `math`   - Vec3 / Mat4 / Aabb / Ray
//! * `rng`    - counter-based hashing and PCG32
//! * `json`   - JSON value, parser and serializer
//! * `color`  - sRGB transfer functions, colour parsing, naming
//! * `diag`   - source spans and structured diagnostics
//! * `par`    - deterministic data-parallel helpers on std threads
#![forbid(unsafe_code)]

pub mod color;
pub mod diag;
pub mod dmath;
pub mod json;
pub mod math;
pub mod par;
pub mod rng;

pub use math::{Aabb, Mat4, Ray, Vec3};
