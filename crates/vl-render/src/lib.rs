//! vl-render: the reference CPU renderer (unidirectional path tracing).
#![forbid(unsafe_code)]

pub mod accel;
pub mod bsdf;
pub mod denoise;
pub mod geom;
pub mod inspect;
pub mod integrator;
pub mod pipeline;
pub mod post;
pub mod rt;
