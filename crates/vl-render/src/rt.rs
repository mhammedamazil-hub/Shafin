//! Runtime form of a scene: acceleration structures, lights, camera, and the
//! surface-shading step that turns a ray hit into a BSDF.
use crate::accel::Bvh;
use crate::bsdf::{Bsdf, DELTA_ROUGHNESS};
use crate::geom::{self, LocalHit, MeshRt};
use vl_core::color::luminance;
use vl_core::dmath;
use vl_core::math::{onb, vec3, Aabb, Mat4, Vec3};
use vl_ir::expr::{Param, ShadeInputs, INPUT_UV};
use vl_ir::expr::{Op, Program};
use vl_ir::scene::{Env, Material, Scene, Shape};
use vl_ir::sky::SkyTable;

pub struct Inst {
    pub shape: Shape,
    pub to_world: Mat4,
    pub to_obj: Mat4,
    pub material: u32,
    pub rand: Vec3,
    pub hidden: bool,
    pub cast_shadow: bool,
    /// Index into `SceneRt::lights`, or -1 when the object is not sampled as a light.
    pub light: i32,
}

pub struct MatRt {
    /// Distinct programs referenced by the material's parameters (bump excluded).
    pub programs: Vec<u32>,
    pub needs_uv: bool,
    pub emissive: bool,
    pub transmissive: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum LightShape {
    Sphere { center: Vec3, radius: f32 },
    /// Parallelogram: centre and the two half-edge vectors.
    Quad { center: Vec3, ex: Vec3, ez: Vec3, n: Vec3, area: f32 },
    /// Ellipse: centre and the two radius vectors.
    Disk { center: Vec3, ex: Vec3, ez: Vec3, n: Vec3, area: f32 },
}

pub struct Light {
    pub shape: LightShape,
    pub obj: u32,
    /// luminance * area, used to choose between lights.
    pub power: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f32,
    pub obj: u32,
    pub local: LocalHit,
}

/// Everything the integrator needs to know about a surface point.
pub struct Surf {
    pub p: Vec3,
    /// Geometric and shading normals, flipped to the side the ray arrived from.
    pub ng: Vec3,
    pub ns: Vec3,
    pub tx: Vec3,
    pub ty: Vec3,
    /// The ray hit the outside of the surface.
    pub front: bool,
    pub bsdf: Bsdf,
    pub emission: Vec3,
    pub obj: u32,
    pub material: u32,
}

impl Surf {
    #[inline]
    pub fn to_local(&self, v: Vec3) -> Vec3 {
        vec3(v.dot(self.tx), v.dot(self.ty), v.dot(self.ns))
    }
    #[inline]
    pub fn to_world(&self, v: Vec3) -> Vec3 {
        self.tx * v.x + self.ty * v.y + self.ns * v.z
    }
}

pub struct CamRt {
    pub pos: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub tan_half: f32,
    pub aspect: f32,
    pub lens_radius: f32,
    pub focus: f32,
    pub blades: u32,
}

impl CamRt {
    /// Ray from the lens centre through film position (px, py): what a pinhole camera sees.
    pub fn pinhole_ray(&self, px: f32, py: f32, w: f32, h: f32) -> (Vec3, Vec3) {
        let x = (2.0 * px / w - 1.0) * self.aspect * self.tan_half;
        let y = (1.0 - 2.0 * py / h) * self.tan_half;
        (self.pos, (self.right * x + self.up * y + self.forward).normalize())
    }

    /// Ray through film position (px, py) in pixels (may be fractional) with lens sample (lu, lv) in [0,1).
    pub fn ray(&self, px: f32, py: f32, w: f32, h: f32, lu: f32, lv: f32) -> (Vec3, Vec3) {
        let (_, dir) = self.pinhole_ray(px, py, w, h);
        if self.lens_radius <= 0.0 {
            return (self.pos, dir);
        }
        let focus_point = self.pos + dir * (self.focus / dir.dot(self.forward));
        let (dx, dy) = if self.blades >= 3 {
            // uniform point in a regular polygon: pick a triangle of the fan, then a point in it
            let n = self.blades as f32;
            let k = (lu * n).floor().min(n - 1.0);
            let u = lu * n - k;
            let (s1, t1) = if u + lv > 1.0 { (1.0 - u, 1.0 - lv) } else { (u, lv) };
            let (sa, ca) = dmath::sincos(dmath::TAU32 * k / n);
            let (sb, cb) = dmath::sincos(dmath::TAU32 * (k + 1.0) / n);
            (s1 * ca + t1 * cb, s1 * sa + t1 * sb)
        } else {
            let r = lu.sqrt();
            let (s, c) = dmath::sincos(dmath::TAU32 * lv);
            (r * c, r * s)
        };
        let o = self.pos + (self.right * dx + self.up * dy) * self.lens_radius;
        (o, (focus_point - o).normalize())
    }

    /// Project a world point to pixel coordinates; `None` if it is behind the camera.
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<(f32, f32, f32)> {
        let v = p - self.pos;
        let z = v.dot(self.forward);
        if z <= 1e-6 {
            return None;
        }
        let x = v.dot(self.right) / z / (self.aspect * self.tan_half);
        let y = v.dot(self.up) / z / self.tan_half;
        Some(((x + 1.0) * 0.5 * w, (1.0 - y) * 0.5 * h, z))
    }
}

pub struct SceneRt<'a> {
    pub scene: &'a Scene,
    pub insts: Vec<Inst>,
    pub bvh: Bvh,
    /// Instances that cannot go into the BVH (infinite planes).
    pub unbounded: Vec<u32>,
    pub mesh_rt: Vec<MeshRt>,
    pub mats: Vec<MatRt>,
    pub lights: Vec<Light>,
    pub sky: Option<SkyTable>,
    pub cam: CamRt,
    /// At least one material lets shadow rays through.
    pub any_transmissive: bool,
    /// Emissive objects that are only found by chance (not sampled as lights): noisy.
    pub unsampled_emitters: Vec<String>,
}

fn program_reads(p: &Program, input: u32) -> bool {
    p.instrs.iter().any(|i| i.op == Op::Input && i.a == input)
}

#[inline]
fn eval_param(p: &Param, regs: &[Vec<Vec3>], progs: &[u32]) -> Vec3 {
    match *p {
        Param::Const(v) => v,
        Param::Expr(pi, r) => {
            let slot = progs.iter().position(|&x| x == pi).unwrap_or(0);
            regs[slot][r as usize]
        }
    }
}

/// Scratch buffers reused across shading calls (one set per thread).
#[derive(Default)]
pub struct Scratch {
    regs: Vec<Vec<Vec3>>,
    bump: Vec<Vec3>,
}

impl<'a> SceneRt<'a> {
    pub fn build(scene: &'a Scene) -> SceneRt<'a> {
        let mesh_rt: Vec<MeshRt> = scene.meshes.iter().map(MeshRt::build).collect();
        let mats: Vec<MatRt> = scene
            .materials
            .iter()
            .map(|m| {
                let mut programs: Vec<u32> = Vec::new();
                for p in [&m.base_color, &m.metallic, &m.roughness, &m.transmission, &m.emission] {
                    if let Param::Expr(pi, _) = p {
                        if !programs.contains(pi) {
                            programs.push(*pi);
                        }
                    }
                }
                let mut needs_uv = programs.iter().any(|&pi| program_reads(&scene.programs[pi as usize], INPUT_UV));
                if let Some((pi, _)) = m.bump {
                    needs_uv |= program_reads(&scene.programs[pi as usize], INPUT_UV);
                }
                let transmissive = match m.transmission {
                    Param::Const(v) => v.x > 0.0,
                    Param::Expr(..) => true,
                };
                MatRt { programs, needs_uv, emissive: m.is_emissive(), transmissive }
            })
            .collect();

        let mut insts = Vec::with_capacity(scene.objects.len());
        let mut lights = Vec::new();
        let mut unsampled = Vec::new();
        for (oi, o) in scene.objects.iter().enumerate() {
            let to_obj = o.xform.inverse().unwrap_or(Mat4::IDENTITY);
            let mut light = -1;
            let m = &scene.materials[o.material as usize];
            if mats[o.material as usize].emissive {
                let lum = match m.emission {
                    Param::Const(c) => luminance(c),
                    Param::Expr(..) => 1.0,
                };
                let x = &o.xform;
                let shape = match o.shape {
                    Shape::Sphere { r } => {
                        let (sx, sy, sz) = (x.vector(Vec3::X).length(), x.vector(Vec3::Y).length(), x.vector(Vec3::Z).length());
                        let uniform = (sx - sy).abs() < 1e-4 * sx && (sx - sz).abs() < 1e-4 * sx;
                        if uniform {
                            Some(LightShape::Sphere { center: x.point(Vec3::ZERO), radius: r * sx })
                        } else {
                            None
                        }
                    }
                    Shape::Rect { hx, hz } => {
                        let (ex, ez) = (x.vector(Vec3::X) * hx, x.vector(Vec3::Z) * hz);
                        let c = ez.cross(ex);
                        Some(LightShape::Quad { center: x.point(Vec3::ZERO), ex, ez, n: c.normalize(), area: 4.0 * c.length() })
                    }
                    Shape::Disk { r } => {
                        let (ex, ez) = (x.vector(Vec3::X) * r, x.vector(Vec3::Z) * r);
                        let c = ez.cross(ex);
                        Some(LightShape::Disk { center: x.point(Vec3::ZERO), ex, ez, n: c.normalize(), area: dmath::PI32 * c.length() })
                    }
                    _ => None,
                };
                match shape {
                    Some(shape) => {
                        let area = match shape {
                            LightShape::Sphere { radius, .. } => dmath::PI32 * radius * radius, // projected area
                            LightShape::Quad { area, .. } | LightShape::Disk { area, .. } => area,
                        };
                        light = lights.len() as i32;
                        lights.push(Light { shape, obj: oi as u32, power: lum * area });
                    }
                    None => unsampled.push(o.name.clone()),
                }
            }
            insts.push(Inst { shape: o.shape.clone(), to_world: o.xform, to_obj, material: o.material, rand: o.rand, hidden: o.hidden, cast_shadow: o.cast_shadow, light });
        }

        let mut bounded = Vec::new();
        let mut unbounded = Vec::new();
        let mut boxes: Vec<Aabb> = Vec::new();
        for (i, o) in scene.objects.iter().enumerate() {
            if o.shape.is_unbounded() {
                unbounded.push(i as u32);
            } else {
                bounded.push(i as u32);
                boxes.push(o.world_bounds(&scene.meshes));
            }
        }
        let mut bvh = Bvh::build(&boxes, 2);
        for p in bvh.prims.iter_mut() {
            *p = bounded[*p as usize];
        }

        let c = &scene.camera;
        let forward = (c.target - c.pos).normalize();
        let right = forward.cross(c.up).normalize();
        let up = right.cross(forward);
        let cam = CamRt {
            pos: c.pos,
            forward,
            right,
            up,
            tan_half: dmath::tan(dmath::radians(c.vfov) * 0.5),
            aspect: scene.film.width as f32 / scene.film.height as f32,
            lens_radius: c.lens_radius,
            focus: c.focus,
            blades: c.blades,
        };
        let sky = match &scene.env {
            Env::Sky(p) => Some(SkyTable::build(p)),
            _ => None,
        };
        let any_transmissive = mats.iter().any(|m| m.transmissive);
        SceneRt { scene, insts, bvh, unbounded, mesh_rt, mats, lights, sky, cam, any_transmissive, unsampled_emitters: unsampled }
    }

    #[inline]
    fn hit_inst(&self, i: u32, o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<LocalHit> {
        let inst = &self.insts[i as usize];
        let (lo, ld) = (inst.to_obj.point(o), inst.to_obj.vector(d));
        geom::intersect(&inst.shape, &self.scene.meshes, &self.mesh_rt, lo, ld, tmin, tmax)
    }

    /// Closest intersection. `camera_ray` skips objects marked hidden.
    pub fn closest(&self, o: Vec3, d: Vec3, tmin: f32, tmax: f32, camera_ray: bool) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        let mut tmax = tmax;
        for &i in &self.unbounded {
            if camera_ray && self.insts[i as usize].hidden {
                continue;
            }
            if let Some(h) = self.hit_inst(i, o, d, tmin, tmax) {
                tmax = h.t;
                best = Some(Hit { t: h.t, obj: i, local: h });
            }
        }
        self.bvh.closest(o, d, tmin, tmax, |i, tm| {
            if camera_ray && self.insts[i as usize].hidden {
                return tm;
            }
            match self.hit_inst(i, o, d, tmin, tm) {
                Some(h) => {
                    best = Some(Hit { t: h.t, obj: i, local: h });
                    h.t
                }
                None => tm,
            }
        });
        best
    }

    /// Is any shadow-casting object (other than `skip`) hit by the segment? Every object blocks.
    pub fn occluded(&self, o: Vec3, d: Vec3, tmin: f32, tmax: f32, skip: u32) -> bool {
        let blocks = |i: u32| i != skip && self.insts[i as usize].cast_shadow && self.hit_inst(i, o, d, tmin, tmax).is_some();
        for &i in &self.unbounded {
            if blocks(i) {
                return true;
            }
        }
        self.bvh.any(o, d, tmin, tmax, blocks)
    }

    /// Fraction of light that travels along a shadow segment. Transmissive surfaces
    /// attenuate instead of blocking (no refraction: "transparent shadows").
    ///
    /// `skip` is the object being sampled as a light (or `u32::MAX`). It must be excluded
    /// rather than avoided with a distance margin: the segment starts from a point that has
    /// been pushed off the surface, so near a light's silhouette, or at a shallow angle to a
    /// panel, any fixed margin is too small and the light would shadow itself.
    pub fn transmittance(&self, o: Vec3, d: Vec3, tmin: f32, tmax: f32, skip: u32, scratch: &mut Scratch) -> Vec3 {
        if !self.any_transmissive {
            return if self.occluded(o, d, tmin, tmax, skip) { Vec3::ZERO } else { Vec3::ONE };
        }
        let mut t = Vec3::ONE;
        let mut start = tmin;
        for _ in 0..32 {
            let hit = match self.closest_shadow(o, d, start, tmax, skip) {
                Some(h) => h,
                None => return t,
            };
            let m = &self.mats[self.insts[hit.obj as usize].material as usize];
            if !m.transmissive {
                return Vec3::ZERO;
            }
            let s = self.shade(o, d, &hit, scratch);
            t = t * s.bsdf.shadow_transmittance(s.ns.dot(d).abs());
            if !s.front {
                t = t * self.absorption(s.material, hit.t - start);
            }
            if t.max_comp() < 1e-4 {
                return Vec3::ZERO;
            }
            start = hit.t + ray_eps(s.p) / d.length().max(1e-20);
        }
        Vec3::ZERO
    }

    fn closest_shadow(&self, o: Vec3, d: Vec3, tmin: f32, tmax: f32, skip: u32) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        let mut tmax = tmax;
        for &i in &self.unbounded {
            if i == skip || !self.insts[i as usize].cast_shadow {
                continue;
            }
            if let Some(h) = self.hit_inst(i, o, d, tmin, tmax) {
                tmax = h.t;
                best = Some(Hit { t: h.t, obj: i, local: h });
            }
        }
        self.bvh.closest(o, d, tmin, tmax, |i, tm| {
            if i == skip || !self.insts[i as usize].cast_shadow {
                return tm;
            }
            match self.hit_inst(i, o, d, tmin, tm) {
                Some(h) => {
                    best = Some(Hit { t: h.t, obj: i, local: h });
                    h.t
                }
                None => tm,
            }
        });
        best
    }

    /// Beer-Lambert transmittance for travelling `dist` metres inside material `mat`.
    pub fn absorption(&self, mat: u32, dist: f32) -> Vec3 {
        let m = &self.scene.materials[mat as usize];
        if m.absorb_dist <= 0.0 || m.absorb_color == Vec3::ONE {
            return Vec3::ONE;
        }
        let k = dist / m.absorb_dist;
        m.absorb_color.map(|c| dmath::pow(c.clamp(1e-6, 1.0), k))
    }

    fn run_programs(&self, mat: &MatRt, inp: &ShadeInputs, scratch: &mut Scratch) {
        if scratch.regs.len() < mat.programs.len() {
            scratch.regs.resize_with(mat.programs.len(), Vec::new);
        }
        for (slot, &pi) in mat.programs.iter().enumerate() {
            self.scene.programs[pi as usize].eval(inp, &mut scratch.regs[slot]);
        }
    }

    /// Evaluate the surface at a hit: normals, material parameters, BSDF, emission.
    pub fn shade(&self, o: Vec3, d: Vec3, hit: &Hit, scratch: &mut Scratch) -> Surf {
        let inst = &self.insts[hit.obj as usize];
        let m: &Material = &self.scene.materials[inst.material as usize];
        let mrt = &self.mats[inst.material as usize];
        let p = o + d * hit.t;
        let p_obj = inst.to_obj.point(o) + inst.to_obj.vector(d) * hit.local.t;
        let (ng_o, mut ns_o) = geom::normals(&inst.shape, &self.scene.meshes, p_obj, &hit.local);
        let uv = if mrt.needs_uv { geom::uv(&inst.shape, &self.scene.meshes, p_obj, &hit.local) } else { Vec3::ZERO };

        if let Some((pi, reg)) = m.bump {
            // finite-difference gradient of the height field along two tangents, in object space
            let prog = &self.scene.programs[pi as usize];
            let (t1, t2) = onb(ns_o);
            let delta = 5.0e-4f32;
            let height = |q: Vec3, scratch: &mut Scratch| {
                let inp = ShadeInputs { p: q, wp: inst.to_world.point(q), uv, n: ns_o, rand: inst.rand };
                prog.eval(&inp, &mut scratch.bump);
                scratch.bump[reg as usize].x
            };
            let h0 = height(p_obj, scratch);
            let h1 = height(p_obj + t1 * delta, scratch);
            let h2 = height(p_obj + t2 * delta, scratch);
            let g = (t1 * (h1 - h0) + t2 * (h2 - h0)) * (m.bump_strength / delta);
            let n = ns_o - g;
            if n.is_finite() && n.length_sq() > 1e-12 {
                ns_o = n.normalize();
            }
        }

        let mut ng = inst.to_obj.normal_t(ng_o).normalize();
        let mut ns = inst.to_obj.normal_t(ns_o).normalize();
        let front = ng.dot(d) < 0.0;
        if !front {
            ng = -ng;
            ns = -ns;
        }
        // a bumped or interpolated normal must not face away from the viewer
        if ns.dot(d) > -1e-3 {
            ns = (ns - d * (ns.dot(d) + 1e-3) / d.length_sq().max(1e-20)).normalize();
            if !ns.is_finite() || ns.dot(ng) <= 0.0 {
                ns = ng;
            }
        }

        let (base, metallic, rough, trans, emission);
        if mrt.programs.is_empty() {
            let c = |p: &Param| p.const_value().unwrap_or(Vec3::ZERO);
            base = c(&m.base_color);
            metallic = c(&m.metallic).x;
            rough = c(&m.roughness).x;
            trans = c(&m.transmission).x;
            emission = c(&m.emission);
        } else {
            let inp = ShadeInputs { p: p_obj, wp: p, uv, n: ns, rand: inst.rand };
            self.run_programs(mrt, &inp, scratch);
            let e = |p: &Param| eval_param(p, &scratch.regs, &mrt.programs);
            base = e(&m.base_color);
            metallic = e(&m.metallic).x;
            rough = e(&m.roughness).x;
            trans = e(&m.transmission).x;
            emission = e(&m.emission);
        }
        let base = base.clamp(0.0, 1.0);
        let rough = rough.clamp(0.0, 1.0);
        let trans = trans.clamp(0.0, 1.0);
        let f0 = ((m.ior - 1.0) / (m.ior + 1.0)).powi(2) * m.specular.clamp(0.0, 1.0);
        let bsdf = Bsdf {
            base,
            metallic: metallic.clamp(0.0, 1.0),
            alpha: (rough * rough).max(1e-4),
            spec_delta: rough < DELTA_ROUGHNESS,
            f0,
            eta: if front || m.thin { m.ior } else { 1.0 / m.ior },
            trans,
            trans_tint: if m.thin { base } else { base.map(|c| c.sqrt()) },
            coat: m.coat.clamp(0.0, 1.0),
            coat_alpha: (m.coat_roughness * m.coat_roughness).max(1e-4),
            coat_delta: m.coat_roughness < DELTA_ROUGHNESS,
            sheen: m.sheen.clamp(0.0, 1.0),
            thin: m.thin,
        };
        let (tx, ty) = onb(ns);
        Surf { p, ng, ns, tx, ty, front, bsdf, emission: emission.max(Vec3::ZERO), obj: hit.obj, material: inst.material }
    }

    /// Emitted radiance of object `obj` at world point `p` (used when a light is sampled directly).
    pub fn emission_at(&self, obj: u32, p: Vec3, scratch: &mut Scratch) -> Vec3 {
        let inst = &self.insts[obj as usize];
        let m = &self.scene.materials[inst.material as usize];
        match m.emission {
            Param::Const(c) => c,
            Param::Expr(..) => {
                let mrt = &self.mats[inst.material as usize];
                let inp = ShadeInputs { p: inst.to_obj.point(p), wp: p, uv: Vec3::ZERO, n: Vec3::Y, rand: inst.rand };
                self.run_programs(mrt, &inp, scratch);
                eval_param(&m.emission, &scratch.regs, &mrt.programs).max(Vec3::ZERO)
            }
        }
    }

    /// Radiance arriving from the environment along direction `d` (unit), without the solar disc.
    pub fn env(&self, d: Vec3) -> Vec3 {
        match &self.scene.env {
            Env::Uniform(c) => *c,
            Env::Gradient { zenith, horizon, ground } => {
                if d.y >= 0.0 {
                    let t = d.y.sqrt();
                    *horizon * (1.0 - t) + *zenith * t
                } else {
                    let t = vl_core::math::smoothstep(0.0, 0.2, -d.y);
                    *horizon * (1.0 - t) + *ground * t
                }
            }
            Env::Sky(_) => self.sky.as_ref().map(|s| s.sky(d)).unwrap_or(Vec3::ZERO),
        }
    }
}

/// Offset used to start secondary rays off a surface.
#[inline]
pub fn ray_eps(p: Vec3) -> f32 {
    1.0e-4 * p.abs().max_comp().max(1.0)
}
