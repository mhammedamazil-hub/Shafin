//! vl-image: raster buffers, codecs, pixel operations, inspection and the
//! layered non-destructive document model. No third-party code.
#![forbid(unsafe_code)]

pub mod deflate;
pub mod image;
pub mod png;

pub use image::{Image, Rect};
