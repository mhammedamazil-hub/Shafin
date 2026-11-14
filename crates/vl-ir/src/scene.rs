//! The scene description every backend consumes.
//!
//! Conventions (also the language's conventions):
//! * right-handed, +Y up, distances in metres, angles in degrees;
//! * radiance in cd/m^2 (nits) per linear-sRGB channel, so real-world light
//!   levels (lux, lumens, EV) can be used directly;
//! * object-to-world transforms are affine; shapes carry their own dimensions.
use crate::expr::{Param, Program};
use vl_core::json::Json;
use vl_core::math::{vec3, Aabb, Mat4, Vec3};

pub const MAX_WIDTH: u32 = 16384;
pub const MAX_HEIGHT: u32 = 16384;
pub const MAX_OBJECTS: usize = 2_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Film {
    pub width: u32,
    pub height: u32,
    /// Maximum samples per pixel.
    pub spp: u32,
    /// Minimum samples per pixel before adaptive sampling may stop a pixel.
    pub min_spp: u32,
    /// Adaptive-sampling relative noise target (0 disables adaptive sampling).
    pub noise_target: f32,
    pub max_depth: u32,
    pub seed: u64,
    /// Upper bound on the luminance (nits, pre-exposure, relative to the exposure white point)
    /// of indirect contributions; suppresses fireflies. 0 disables.
    pub clamp_indirect: f32,
}

impl Default for Film {
    fn default() -> Film {
        Film { width: 1280, height: 720, spp: 64, min_spp: 16, noise_target: 0.0, max_depth: 12, seed: 1, clamp_indirect: 20.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub pos: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    /// Vertical field of view in degrees.
    pub vfov: f32,
    /// Lens radius in metres (0 = pinhole, everything sharp).
    pub lens_radius: f32,
    /// Distance to the plane of perfect focus, metres.
    pub focus: f32,
    /// Number of aperture blades (0 = circular).
    pub blades: u32,
}

impl Default for Camera {
    fn default() -> Camera {
        Camera { pos: vec3(0.0, 1.5, 4.0), target: vec3(0.0, 0.5, 0.0), up: Vec3::Y, vfov: 35.0, lens_radius: 0.0, focus: 4.0, blades: 0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkyParams {
    /// Unit vector pointing from the scene towards the sun.
    pub sun_dir: Vec3,
    /// 2 = very clear, 3 = clear, 6 = hazy, 10 = very hazy.
    pub turbidity: f32,
    pub ground_albedo: Vec3,
    /// Multiplier on the sun's angular radius (1 = real sun, larger = softer shadows).
    pub sun_size: f32,
    /// Multipliers on sun and sky radiance (1 = physical).
    pub sun_strength: f32,
    pub sky_strength: f32,
}

impl Default for SkyParams {
    fn default() -> SkyParams {
        SkyParams { sun_dir: vec3(0.35, 0.72, 0.6).normalize(), turbidity: 3.0, ground_albedo: Vec3::splat(0.25), sun_size: 1.0, sun_strength: 1.0, sky_strength: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Env {
    /// Constant radiance from every direction.
    Uniform(Vec3),
    /// Vertical gradient: zenith -> horizon -> ground.
    Gradient { zenith: Vec3, horizon: Vec3, ground: Vec3 },
    /// Physical sun and sky.
    Sky(SkyParams),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub base_color: Param,
    pub metallic: Param,
    pub roughness: Param,
    /// Weight of the dielectric specular layer (1 = Fresnel from `ior`).
    pub specular: f32,
    pub ior: f32,
    /// 0 = opaque, 1 = fully refractive (glass, water).
    pub transmission: Param,
    /// Clear-coat layer weight and roughness (varnish, car paint, glazed ceramic).
    pub coat: f32,
    pub coat_roughness: f32,
    /// Emitted radiance in nits (linear RGB).
    pub emission: Param,
    /// Retro-reflective fuzz for cloth (0..1).
    pub sheen: f32,
    /// Optional height field (program, register) in metres for bump mapping, and its strength.
    pub bump: Option<(u32, u32)>,
    pub bump_strength: f32,
    /// Beer-Lambert absorption inside a transmissive object: transmittance colour reached after `absorb_dist` metres.
    pub absorb_color: Vec3,
    pub absorb_dist: f32,
    /// Thin-walled (no refraction offset; e.g. window pane, soap film, paper).
    pub thin: bool,
}

impl Default for Material {
    fn default() -> Material {
        Material {
            name: String::new(),
            base_color: Param::Const(Vec3::splat(0.6)),
            metallic: Param::scalar(0.0),
            roughness: Param::scalar(0.5),
            specular: 1.0,
            ior: 1.5,
            transmission: Param::scalar(0.0),
            coat: 0.0,
            coat_roughness: 0.03,
            emission: Param::Const(Vec3::ZERO),
            sheen: 0.0,
            bump: None,
            bump_strength: 1.0,
            absorb_color: Vec3::ONE,
            absorb_dist: 0.0,
            thin: false,
        }
    }
}

impl Material {
    pub fn is_emissive(&self) -> bool {
        match self.emission {
            Param::Const(c) => c.max_comp() > 0.0,
            Param::Expr(..) => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    /// Per-vertex shading normals (same length as `positions`) or empty for flat shading.
    pub normals: Vec<Vec3>,
    /// Per-vertex UV (z unused) or empty.
    pub uvs: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
}

impl Mesh {
    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for p in &self.positions {
            b.grow(*p);
        }
        b
    }
}

/// Geometry in object space. Dimensions are in metres before the object transform.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Sphere of radius `r` centred on the origin.
    Sphere { r: f32 },
    /// Axis-aligned box with half-extents `half`; `round` > 0 rounds every edge with that radius.
    Box { half: Vec3, round: f32 },
    /// Capped truncated cone around the Y axis: radius `r0` at y = -half_h and `r1` at y = +half_h
    /// (a cylinder when r0 == r1, a cone when r1 == 0).
    Cone { r0: f32, r1: f32, half_h: f32 },
    /// Torus around the Y axis: ring radius `major`, tube radius `minor`.
    Torus { major: f32, minor: f32 },
    /// Capsule along Y: cylinder of radius `r` and half-length `half_h` with hemispherical ends.
    Capsule { r: f32, half_h: f32 },
    /// Disk of radius `r` in the y = 0 plane, facing +Y.
    Disk { r: f32 },
    /// Rectangle in the y = 0 plane with half-extents (hx, hz), facing +Y.
    Rect { hx: f32, hz: f32 },
    /// Infinite plane y = 0, facing +Y.
    Plane,
    /// Triangle mesh (index into `Scene::meshes`).
    Mesh(u32),
}

impl Shape {
    pub fn kind(&self) -> &'static str {
        match self {
            Shape::Sphere { .. } => "sphere",
            Shape::Box { .. } => "box",
            Shape::Cone { r0, r1, .. } => {
                if r0 == r1 {
                    "cylinder"
                } else {
                    "cone"
                }
            }
            Shape::Torus { .. } => "torus",
            Shape::Capsule { .. } => "capsule",
            Shape::Disk { .. } => "disk",
            Shape::Rect { .. } => "rect",
            Shape::Plane => "plane",
            Shape::Mesh(_) => "mesh",
        }
    }

    /// Object-space bounds. The infinite plane reports a large finite slab so that
    /// placement and framing code can treat it uniformly; use `is_unbounded` to exclude it.
    pub fn local_bounds(&self, meshes: &[Mesh]) -> Aabb {
        match self {
            Shape::Sphere { r } => Aabb::new(Vec3::splat(-*r), Vec3::splat(*r)),
            Shape::Box { half, .. } => Aabb::new(-*half, *half),
            Shape::Cone { r0, r1, half_h } => {
                let r = r0.max(*r1);
                Aabb::new(vec3(-r, -*half_h, -r), vec3(r, *half_h, r))
            }
            Shape::Torus { major, minor } => {
                let r = major + minor;
                Aabb::new(vec3(-r, -*minor, -r), vec3(r, *minor, r))
            }
            Shape::Capsule { r, half_h } => Aabb::new(vec3(-*r, -(half_h + r), -*r), vec3(*r, half_h + r, *r)),
            Shape::Disk { r } => Aabb::new(vec3(-*r, 0.0, -*r), vec3(*r, 0.0, *r)),
            Shape::Rect { hx, hz } => Aabb::new(vec3(-*hx, 0.0, -*hz), vec3(*hx, 0.0, *hz)),
            Shape::Plane => Aabb::new(vec3(-1.0e5, 0.0, -1.0e5), vec3(1.0e5, 0.0, 1.0e5)),
            Shape::Mesh(i) => meshes.get(*i as usize).map(|m| m.bounds()).unwrap_or(Aabb::EMPTY),
        }
    }

    pub fn is_unbounded(&self) -> bool {
        matches!(self, Shape::Plane)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    /// Unique, human-meaningful name; inspection reports and edits refer to it.
    pub name: String,
    pub shape: Shape,
    /// Object-to-world transform.
    pub xform: Mat4,
    pub material: u32,
    /// Per-object random numbers in [0,1), available to shading programs as `RAND`.
    pub rand: Vec3,
    /// Invisible to camera rays (still lights the scene / casts shadows).
    pub hidden: bool,
    pub cast_shadow: bool,
}

impl Object {
    pub fn world_bounds(&self, meshes: &[Mesh]) -> Aabb {
        self.shape.local_bounds(meshes).transformed(&self.xform)
    }
}

/// Homogeneous participating medium filling the whole scene (haze, fog, dusty air).
#[derive(Clone, Debug, PartialEq)]
pub struct Fog {
    /// Scattering coefficient per metre (linear RGB).
    pub scatter: Vec3,
    /// Absorption coefficient per metre.
    pub absorb: Vec3,
    /// Henyey-Greenstein anisotropy, -1..1 (0.6-0.8 for haze).
    pub g: f32,
    /// Exponential falloff with height: density = exp(-(y - base) / height) when height > 0.
    pub height: f32,
    pub base: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tonemap {
    /// Clamp only (scene-referred values above 1 clip).
    Linear,
    /// Filmic curve with highlight roll-off and gentle toe (default, photographic).
    Filmic,
    /// Reinhard on luminance.
    Reinhard,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    /// `None` = automatic exposure; `Some(ev)` = EV at ISO 100 (15 = sunny day, 7 = living room at night).
    pub exposure_ev: Option<f32>,
    /// Stops added on top of the (automatic or explicit) exposure.
    pub exposure_bias: f32,
    pub tonemap: Tonemap,
    /// White balance in kelvin (6500 = neutral).
    pub white_balance: f32,
    pub denoise: bool,
    pub bloom: f32,
    pub vignette: f32,
    pub grain: f32,
    pub saturation: f32,
    pub contrast: f32,
}

impl Default for Post {
    fn default() -> Post {
        Post { exposure_ev: None, exposure_bias: 0.0, tonemap: Tonemap::Filmic, white_balance: 6500.0, denoise: true, bloom: 0.0, vignette: 0.0, grain: 0.0, saturation: 1.0, contrast: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub film: Film,
    pub camera: Camera,
    pub env: Env,
    pub materials: Vec<Material>,
    pub programs: Vec<Program>,
    pub meshes: Vec<Mesh>,
    pub objects: Vec<Object>,
    pub fog: Option<Fog>,
    pub post: Post,
}

impl Default for Scene {
    fn default() -> Scene {
        Scene {
            film: Film::default(),
            camera: Camera::default(),
            env: Env::Sky(SkyParams::default()),
            materials: vec![Material { name: "default".into(), ..Material::default() }],
            programs: Vec::new(),
            meshes: Vec::new(),
            objects: Vec::new(),
            fog: None,
            post: Post::default(),
        }
    }
}

fn finite3(v: Vec3) -> bool {
    v.is_finite()
}

impl Scene {
    /// Bounds of all bounded objects (the infinite plane is excluded).
    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for o in &self.objects {
            if !o.shape.is_unbounded() {
                b = b.union(&o.world_bounds(&self.meshes));
            }
        }
        b
    }

    pub fn triangle_count(&self) -> usize {
        self.objects.iter().map(|o| if let Shape::Mesh(i) = o.shape { self.meshes[i as usize].indices.len() } else { 0 }).sum()
    }

    /// Structural validation. A scene that passes cannot make a backend index out of
    /// range, divide by zero in set-up code, or allocate without bound.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut e = Vec::new();
        let f = &self.film;
        if f.width == 0 || f.height == 0 || f.width > MAX_WIDTH || f.height > MAX_HEIGHT {
            e.push(format!("film size {}x{} is outside 1..{MAX_WIDTH} x 1..{MAX_HEIGHT}", f.width, f.height));
        }
        if f.spp == 0 || f.spp > 65536 {
            e.push(format!("samples per pixel {} is outside 1..65536", f.spp));
        }
        if f.max_depth == 0 || f.max_depth > 64 {
            e.push(format!("max_depth {} is outside 1..64", f.max_depth));
        }
        let c = &self.camera;
        if !finite3(c.pos) || !finite3(c.target) || !finite3(c.up) {
            e.push("camera has a non-finite position, target or up vector".into());
        } else {
            let d = c.target - c.pos;
            if d.length() < 1e-6 {
                e.push("camera position and target coincide".into());
            } else if d.normalize().cross(c.up).length() < 1e-4 {
                e.push("camera direction is parallel to its up vector".into());
            }
        }
        if !(c.vfov > 0.1 && c.vfov < 170.0) {
            e.push(format!("camera field of view {} is outside 0.1..170 degrees", c.vfov));
        }
        if !(c.lens_radius >= 0.0 && c.lens_radius.is_finite()) || !(c.focus > 0.0 && c.focus.is_finite()) {
            e.push("camera lens radius must be >= 0 and focus distance > 0".into());
        }
        if let Env::Sky(s) = &self.env {
            if !finite3(s.sun_dir) || (s.sun_dir.length() - 1.0).abs() > 1e-3 {
                e.push("sky sun direction must be a unit vector".into());
            }
            if !(s.turbidity >= 1.0 && s.turbidity <= 20.0) {
                e.push(format!("sky turbidity {} is outside 1..20", s.turbidity));
            }
        }
        for (i, p) in self.programs.iter().enumerate() {
            if let Err(m) = p.validate() {
                e.push(format!("program {i}: {m}"));
            }
        }
        let check_param = |p: &Param, what: &str, mat: &str, e: &mut Vec<String>| match p {
            Param::Const(v) => {
                if !v.is_finite() {
                    e.push(format!("material '{mat}': {what} is not finite"));
                }
            }
            Param::Expr(pi, r) => {
                if self.programs.get(*pi as usize).map(|p| (*r as usize) < p.len()) != Some(true) {
                    e.push(format!("material '{mat}': {what} refers to a missing program register"));
                }
            }
        };
        for m in &self.materials {
            check_param(&m.base_color, "base_color", &m.name, &mut e);
            check_param(&m.metallic, "metallic", &m.name, &mut e);
            check_param(&m.roughness, "roughness", &m.name, &mut e);
            check_param(&m.transmission, "transmission", &m.name, &mut e);
            check_param(&m.emission, "emission", &m.name, &mut e);
            if let Some((pi, r)) = m.bump {
                check_param(&Param::Expr(pi, r), "bump", &m.name, &mut e);
            }
            if !(m.ior >= 1.0 && m.ior <= 4.0) {
                e.push(format!("material '{}': ior {} is outside 1..4", m.name, m.ior));
            }
        }
        if self.objects.len() > MAX_OBJECTS {
            e.push(format!("{} objects exceed the limit of {MAX_OBJECTS}", self.objects.len()));
        }
        for m in &self.meshes {
            let n = m.positions.len();
            if (!m.normals.is_empty() && m.normals.len() != n) || (!m.uvs.is_empty() && m.uvs.len() != n) {
                e.push("mesh attribute arrays have mismatched lengths".into());
            }
            if m.indices.iter().any(|t| t.iter().any(|&i| i as usize >= n)) {
                e.push("mesh index out of range".into());
            }
            if m.positions.iter().any(|p| !p.is_finite()) {
                e.push("mesh has a non-finite vertex".into());
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for o in &self.objects {
            if !names.insert(o.name.as_str()) {
                e.push(format!("duplicate object name '{}'", o.name));
            }
            if o.material as usize >= self.materials.len() {
                e.push(format!("object '{}' refers to a missing material", o.name));
            }
            if let Shape::Mesh(i) = o.shape {
                if i as usize >= self.meshes.len() {
                    e.push(format!("object '{}' refers to a missing mesh", o.name));
                }
            }
            if o.xform.0.iter().flatten().any(|v| !v.is_finite()) || o.xform.inverse().is_none() {
                e.push(format!("object '{}' has a non-invertible or non-finite transform", o.name));
            }
            let dims_ok = match &o.shape {
                Shape::Sphere { r } => *r > 0.0 && r.is_finite(),
                Shape::Box { half, round } => half.min_comp() > 0.0 && half.is_finite() && *round >= 0.0 && *round <= half.min_comp(),
                Shape::Cone { r0, r1, half_h } => *r0 >= 0.0 && *r1 >= 0.0 && r0.max(*r1) > 0.0 && *half_h > 0.0 && (r0 + r1 + half_h).is_finite(),
                Shape::Torus { major, minor } => *minor > 0.0 && *major > 0.0 && (major + minor).is_finite(),
                Shape::Capsule { r, half_h } => *r > 0.0 && *half_h >= 0.0 && (r + half_h).is_finite(),
                Shape::Disk { r } => *r > 0.0 && r.is_finite(),
                Shape::Rect { hx, hz } => *hx > 0.0 && *hz > 0.0 && (hx + hz).is_finite(),
                Shape::Plane | Shape::Mesh(_) => true,
            };
            if !dims_ok {
                e.push(format!("object '{}' ({}) has invalid dimensions", o.name, o.shape.kind()));
            }
        }
        if e.is_empty() {
            Ok(())
        } else {
            Err(e)
        }
    }

    /// Compact machine-readable summary (not a full serialisation).
    pub fn summary_json(&self) -> Json {
        let b = self.bounds();
        let v3 = |v: Vec3| Json::nums(&[v.x as f64, v.y as f64, v.z as f64], 4);
        let mut j = Json::obj()
            .with("width", self.film.width as f64)
            .with("height", self.film.height as f64)
            .with("samples", self.film.spp as f64)
            .with("objects", self.objects.len() as f64)
            .with("materials", self.materials.len() as f64)
            .with("triangles", self.triangle_count() as f64)
            .with("camera", Json::obj().with("position", v3(self.camera.pos)).with("target", v3(self.camera.target)).with("fov", Json::round(self.camera.vfov as f64, 3)));
        if !b.is_empty() {
            j.set("bounds_min", v3(b.min));
            j.set("bounds_max", v3(b.max));
        }
        j
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scene_is_valid_and_bad_scenes_are_rejected() {
        let mut s = Scene::default();
        s.objects.push(Object { name: "ball".into(), shape: Shape::Sphere { r: 0.5 }, xform: Mat4::translate(vec3(0.0, 0.5, 0.0)), material: 0, rand: Vec3::ZERO, hidden: false, cast_shadow: true });
        s.validate().unwrap();
        assert_eq!(s.bounds().min, vec3(-0.5, 0.0, -0.5));

        let mut bad = s.clone();
        bad.objects[0].material = 9;
        bad.objects.push(bad.objects[0].clone());
        bad.film.width = 0;
        bad.camera.target = bad.camera.pos;
        let errs = bad.validate().unwrap_err();
        assert!(errs.iter().any(|m| m.contains("missing material")));
        assert!(errs.iter().any(|m| m.contains("duplicate object name")));
        assert!(errs.iter().any(|m| m.contains("film size")));
        assert!(errs.iter().any(|m| m.contains("coincide")));

        let mut bad = s.clone();
        bad.objects[0].shape = Shape::Sphere { r: f32::NAN };
        bad.objects[0].xform = Mat4::scale(vec3(1.0, 0.0, 1.0));
        let errs = bad.validate().unwrap_err();
        assert!(errs.iter().any(|m| m.contains("invalid dimensions")) && errs.iter().any(|m| m.contains("non-invertible")));
    }
}
