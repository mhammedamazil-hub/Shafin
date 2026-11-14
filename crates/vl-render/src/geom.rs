//! Ray intersection for every shape in the IR.
//!
//! Rays are given in OBJECT space with a direction that is NOT normalised, so the
//! returned parameter `t` is identical to the world-space parameter. Analytic
//! solutions are used where they are robust (sphere, box, cone/cylinder, disk,
//! rect, plane, triangles); rounded boxes, tori and capsules have exact signed
//! distance functions and are sphere-traced.
use crate::accel::Bvh;
use vl_core::dmath;
use vl_core::math::{vec3, Aabb, Vec3};
use vl_ir::scene::{Mesh, Shape};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalHit {
    pub t: f32,
    /// Sub-primitive: triangle index (mesh), face code (box, cone), otherwise 0.
    pub prim: u32,
    /// Barycentric coordinates (mesh only).
    pub u: f32,
    pub v: f32,
}

pub struct MeshRt {
    pub bvh: Bvh,
}

impl MeshRt {
    pub fn build(m: &Mesh) -> MeshRt {
        let bounds: Vec<Aabb> = m
            .indices
            .iter()
            .map(|t| {
                let mut b = Aabb::EMPTY;
                for &i in t {
                    b.grow(m.positions[i as usize]);
                }
                b
            })
            .collect();
        MeshRt { bvh: Bvh::build(&bounds, 4) }
    }
}

const SDF_STEPS: usize = 160;

/// Signed distance for the sphere-traced shapes (exact Euclidean distance).
#[inline]
pub fn sdf(shape: &Shape, p: Vec3) -> f32 {
    match *shape {
        Shape::Box { half, round } => {
            let q = p.abs() - (half - Vec3::splat(round));
            q.max(Vec3::ZERO).length() + q.max_comp().min(0.0) - round
        }
        Shape::Torus { major, minor } => {
            let ring = (p.x * p.x + p.z * p.z).sqrt() - major;
            (ring * ring + p.y * p.y).sqrt() - minor
        }
        Shape::Capsule { r, half_h } => {
            let y = p.y - p.y.clamp(-half_h, half_h);
            (p.x * p.x + y * y + p.z * p.z).sqrt() - r
        }
        _ => f32::MAX,
    }
}

fn sdf_normal(shape: &Shape, p: Vec3) -> Vec3 {
    match *shape {
        Shape::Box { half, round } => {
            let q = p.abs() - (half - Vec3::splat(round));
            let m = q.max(Vec3::ZERO);
            let sgn = vec3(if p.x < 0.0 { -1.0 } else { 1.0 }, if p.y < 0.0 { -1.0 } else { 1.0 }, if p.z < 0.0 { -1.0 } else { 1.0 });
            if m.length_sq() > 0.0 {
                (m * sgn).normalize()
            } else {
                let a = q.argmax(); // the face we are closest to (all components are <= 0 here)
                let mut n = Vec3::ZERO;
                match a {
                    0 => n.x = sgn.x,
                    1 => n.y = sgn.y,
                    _ => n.z = sgn.z,
                }
                n
            }
        }
        Shape::Torus { major, .. } => {
            let l = (p.x * p.x + p.z * p.z).sqrt().max(1e-20);
            (p - vec3(p.x / l * major, 0.0, p.z / l * major)).normalize()
        }
        Shape::Capsule { half_h, .. } => (p - vec3(0.0, p.y.clamp(-half_h, half_h), 0.0)).normalize(),
        _ => Vec3::Y,
    }
}

fn trace_sdf(shape: &Shape, bounds: &Aabb, o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<f32> {
    let len = d.length();
    if !(len > 0.0) {
        return None;
    }
    let inv = vec3(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
    let t0 = (bounds.min - o) * inv;
    let t1 = (bounds.max - o) * inv;
    let lo = t0.min(t1);
    let hi = t0.max(t1);
    let extent = (bounds.max - bounds.min).max_comp();
    let eps = (extent * 4.0e-6).max(1.0e-7);
    // widen the slab slightly so grazing rays are not clipped by rounding
    let pad = eps / len * 4.0;
    let start = lo.max_comp().max(tmin) - pad;
    let end = hi.min_comp().min(tmax) + pad;
    if start > end {
        return None;
    }
    let mut t = start.max(tmin);
    let mut s = sdf(shape, o + d * t);
    if s.abs() < eps {
        // the origin sits on the surface: step off before deciding which side we are on
        t += 4.0 * eps / len;
        s = sdf(shape, o + d * t);
    }
    let sign = if s < 0.0 { -1.0 } else { 1.0 };
    for _ in 0..SDF_STEPS {
        let dist = sign * s;
        if dist < eps {
            return if t > tmin && t < tmax { Some(t) } else { None };
        }
        t += dist / len;
        if t > end {
            return None;
        }
        s = sdf(shape, o + d * t);
    }
    None
}

#[inline]
fn hit_sphere(r: f32, o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<f32> {
    let a = d.length_sq();
    let b = o.dot(d);
    let c = o.length_sq() - r * r;
    let disc = b * b - a * c;
    if !(disc >= 0.0) {
        return None;
    }
    let sq = disc.sqrt();
    let q = if b < 0.0 { -b + sq } else { -b - sq };
    let (mut t0, mut t1) = (q / a, c / q);
    if t0 > t1 {
        std::mem::swap(&mut t0, &mut t1);
    }
    if t0 > tmin && t0 < tmax {
        Some(t0)
    } else if t1 > tmin && t1 < tmax {
        Some(t1)
    } else {
        None
    }
}

#[inline]
fn hit_box(half: Vec3, o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<(f32, u32)> {
    let inv = vec3(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
    let t0 = (-half - o) * inv;
    let t1 = (half - o) * inv;
    let lo = t0.min(t1);
    let hi = t0.max(t1);
    let tn = lo.max_comp();
    let tf = hi.min_comp();
    if !(tn <= tf) {
        return None;
    }
    // entry: the slab whose near plane is crossed LAST; exit: the slab whose far plane is crossed FIRST
    let (t, axis, outward_positive) = if tn > tmin {
        let axis = lo.argmax();
        (tn, axis, d[axis] < 0.0)
    } else {
        let axis = hi.argmin();
        (tf, axis, d[axis] > 0.0)
    };
    if t > tmin && t < tmax {
        Some((t, (axis * 2) as u32 + outward_positive as u32))
    } else {
        None
    }
}

fn hit_cone(r0: f32, r1: f32, h: f32, o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<(f32, u32)> {
    let k = (r1 - r0) / (2.0 * h);
    let rm = 0.5 * (r0 + r1);
    let mut best = tmax;
    let mut code = u32::MAX;
    let ro = rm + k * o.y;
    let a = d.x * d.x + d.z * d.z - k * k * d.y * d.y;
    let b = o.x * d.x + o.z * d.z - k * ro * d.y;
    let c = o.x * o.x + o.z * o.z - ro * ro;
    let mut side = |t: f32| {
        if t > tmin && t < best {
            let y = o.y + t * d.y;
            if y.abs() <= h && rm + k * y >= 0.0 {
                best = t;
                code = 0;
            }
        }
    };
    if a.abs() > 1e-12 {
        let disc = b * b - a * c;
        if disc >= 0.0 {
            let sq = disc.sqrt();
            let q = if b < 0.0 { -b + sq } else { -b - sq };
            side(q / a);
            if q != 0.0 {
                side(c / q);
            }
        }
    } else if b.abs() > 1e-12 {
        side(-c / (2.0 * b));
    }
    if d.y != 0.0 {
        for (yc, rc, cap) in [(-h, r0, 1u32), (h, r1, 2u32)] {
            if rc > 0.0 {
                let t = (yc - o.y) / d.y;
                if t > tmin && t < best {
                    let (x, z) = (o.x + t * d.x, o.z + t * d.z);
                    if x * x + z * z <= rc * rc {
                        best = t;
                        code = cap;
                    }
                }
            }
        }
    }
    if code == u32::MAX {
        None
    } else {
        Some((best, code))
    }
}

#[inline]
fn hit_triangle(p0: Vec3, p1: Vec3, p2: Vec3, o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<(f32, f32, f32)> {
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    if det == 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = o - p0;
    let u = tv.dot(pv) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = tv.cross(e1);
    let v = d.dot(qv) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(qv) * inv;
    if t > tmin && t < tmax {
        Some((t, u, v))
    } else {
        None
    }
}

/// Closest intersection of an object-space ray with `shape` in (tmin, tmax).
pub fn intersect(shape: &Shape, meshes: &[Mesh], mesh_rt: &[MeshRt], o: Vec3, d: Vec3, tmin: f32, tmax: f32) -> Option<LocalHit> {
    let hit = |t: f32, prim: u32| LocalHit { t, prim, u: 0.0, v: 0.0 };
    match *shape {
        Shape::Sphere { r } => hit_sphere(r, o, d, tmin, tmax).map(|t| hit(t, 0)),
        Shape::Box { half, round } => {
            if round > 0.0 {
                trace_sdf(shape, &Aabb::new(-half, half), o, d, tmin, tmax).map(|t| hit(t, 0))
            } else {
                hit_box(half, o, d, tmin, tmax).map(|(t, f)| hit(t, f))
            }
        }
        Shape::Cone { r0, r1, half_h } => hit_cone(r0, r1, half_h, o, d, tmin, tmax).map(|(t, f)| hit(t, f)),
        Shape::Torus { .. } | Shape::Capsule { .. } => trace_sdf(shape, &shape.local_bounds(meshes), o, d, tmin, tmax).map(|t| hit(t, 0)),
        Shape::Disk { r } => {
            let t = -o.y / d.y;
            if t > tmin && t < tmax {
                let (x, z) = (o.x + t * d.x, o.z + t * d.z);
                if x * x + z * z <= r * r {
                    return Some(hit(t, 0));
                }
            }
            None
        }
        Shape::Rect { hx, hz } => {
            let t = -o.y / d.y;
            if t > tmin && t < tmax {
                let (x, z) = (o.x + t * d.x, o.z + t * d.z);
                if x.abs() <= hx && z.abs() <= hz {
                    return Some(hit(t, 0));
                }
            }
            None
        }
        Shape::Plane => {
            let t = -o.y / d.y;
            if t > tmin && t < tmax {
                Some(hit(t, 0))
            } else {
                None
            }
        }
        Shape::Mesh(mi) => {
            let m = &meshes[mi as usize];
            let mut best: Option<LocalHit> = None;
            mesh_rt[mi as usize].bvh.closest(o, d, tmin, tmax, |ti, tm| {
                let [a, b, c] = m.indices[ti as usize];
                match hit_triangle(m.positions[a as usize], m.positions[b as usize], m.positions[c as usize], o, d, tmin, tm) {
                    Some((t, u, v)) => {
                        best = Some(LocalHit { t, prim: ti, u, v });
                        t
                    }
                    None => tm,
                }
            });
            best
        }
    }
}

/// Geometric normal, shading normal (both object space, outward, unit length) at a hit point.
pub fn normals(shape: &Shape, meshes: &[Mesh], p: Vec3, h: &LocalHit) -> (Vec3, Vec3) {
    match *shape {
        Shape::Sphere { .. } => {
            let n = p.normalize();
            (n, n)
        }
        Shape::Box { round, .. } => {
            if round > 0.0 {
                let n = sdf_normal(shape, p);
                (n, n)
            } else {
                let mut n = Vec3::ZERO;
                let s = if h.prim & 1 == 1 { 1.0 } else { -1.0 };
                match h.prim / 2 {
                    0 => n.x = s,
                    1 => n.y = s,
                    _ => n.z = s,
                }
                (n, n)
            }
        }
        Shape::Cone { r0, r1, half_h } => {
            let n = match h.prim {
                1 => vec3(0.0, -1.0, 0.0),
                2 => Vec3::Y,
                _ => {
                    let k = (r1 - r0) / (2.0 * half_h);
                    let rm = 0.5 * (r0 + r1);
                    vec3(p.x, -k * (rm + k * p.y), p.z).normalize()
                }
            };
            (n, n)
        }
        Shape::Torus { .. } | Shape::Capsule { .. } => {
            let n = sdf_normal(shape, p);
            (n, n)
        }
        Shape::Disk { .. } | Shape::Rect { .. } | Shape::Plane => (Vec3::Y, Vec3::Y),
        Shape::Mesh(mi) => {
            let m = &meshes[mi as usize];
            let [a, b, c] = m.indices[h.prim as usize];
            let (p0, p1, p2) = (m.positions[a as usize], m.positions[b as usize], m.positions[c as usize]);
            let ng = (p1 - p0).cross(p2 - p0).normalize();
            if m.normals.is_empty() {
                (ng, ng)
            } else {
                let w = 1.0 - h.u - h.v;
                let ns = (m.normals[a as usize] * w + m.normals[b as usize] * h.u + m.normals[c as usize] * h.v).normalize();
                // keep the geometric normal on the same side as the shading normal
                (if ng.dot(ns) < 0.0 { -ng } else { ng }, ns)
            }
        }
    }
}

/// Surface parameterisation (u, v in [0,1] where the shape has a natural one).
pub fn uv(shape: &Shape, meshes: &[Mesh], p: Vec3, h: &LocalHit) -> Vec3 {
    let around = |x: f32, z: f32| dmath::atan2(z, x) * (0.5 * dmath::INV_PI32) + 0.5;
    match *shape {
        Shape::Sphere { r } => vec3(around(p.x, p.z), dmath::acos((p.y / r).clamp(-1.0, 1.0)) * dmath::INV_PI32, 0.0),
        Shape::Box { half, .. } => {
            let q = vec3(p.x / half.x, p.y / half.y, p.z / half.z) * 0.5 + Vec3::splat(0.5);
            let a = (p.abs() / half).max_dim();
            match a {
                0 => vec3(q.z, q.y, 0.0),
                1 => vec3(q.x, q.z, 0.0),
                _ => vec3(q.x, q.y, 0.0),
            }
        }
        Shape::Cone { half_h, .. } | Shape::Capsule { half_h, .. } => vec3(around(p.x, p.z), (p.y / half_h.max(1e-9)) * 0.5 + 0.5, 0.0),
        Shape::Torus { major, .. } => {
            let l = (p.x * p.x + p.z * p.z).sqrt();
            vec3(around(p.x, p.z), dmath::atan2(p.y, l - major) * (0.5 * dmath::INV_PI32) + 0.5, 0.0)
        }
        Shape::Disk { r } => vec3(p.x / r * 0.5 + 0.5, p.z / r * 0.5 + 0.5, 0.0),
        Shape::Rect { hx, hz } => vec3(p.x / hx * 0.5 + 0.5, p.z / hz * 0.5 + 0.5, 0.0),
        Shape::Plane => vec3(p.x, p.z, 0.0),
        Shape::Mesh(mi) => {
            let m = &meshes[mi as usize];
            if m.uvs.is_empty() {
                return Vec3::ZERO;
            }
            let [a, b, c] = m.indices[h.prim as usize];
            m.uvs[a as usize] * (1.0 - h.u - h.v) + m.uvs[b as usize] * h.u + m.uvs[c as usize] * h.v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::rng::Rng;

    /// Inside/outside function written independently of the intersection code.
    fn implicit(shape: &Shape, p: Vec3) -> f32 {
        match *shape {
            Shape::Sphere { r } => p.length() - r,
            Shape::Box { half, round } if round == 0.0 => (p.abs() - half).max_comp(),
            Shape::Cone { r0, r1, half_h } => {
                let radius = r0 + (r1 - r0) * (p.y + half_h) / (2.0 * half_h);
                ((p.x * p.x + p.z * p.z).sqrt() - radius).max(p.y.abs() - half_h)
            }
            _ => sdf(shape, p),
        }
    }

    /// Reference first crossing: dense sampling for a sign change, then bisection.
    fn reference_hit(shape: &Shape, o: Vec3, d: Vec3, tmax: f32) -> Option<f32> {
        let n = 40_000;
        let f = |t: f32| implicit(shape, o + d * t);
        let mut prev = f(0.0);
        for i in 1..=n {
            let t = tmax * i as f32 / n as f32;
            let cur = f(t);
            if (prev < 0.0) != (cur < 0.0) {
                let (mut a, mut b) = (tmax * (i - 1) as f32 / n as f32, t);
                for _ in 0..40 {
                    let m = 0.5 * (a + b);
                    if (f(m) < 0.0) == (prev < 0.0) {
                        a = m;
                    } else {
                        b = m;
                    }
                }
                return Some(0.5 * (a + b));
            }
            prev = cur;
        }
        None
    }

    #[test]
    fn shapes_agree_with_dense_reference() {
        let shapes = [
            Shape::Sphere { r: 0.7 },
            Shape::Box { half: vec3(0.5, 0.3, 0.8), round: 0.0 },
            Shape::Box { half: vec3(0.5, 0.3, 0.8), round: 0.1 },
            Shape::Cone { r0: 0.4, r1: 0.4, half_h: 0.6 },
            Shape::Cone { r0: 0.6, r1: 0.0, half_h: 0.5 },
            Shape::Cone { r0: 0.2, r1: 0.5, half_h: 0.4 },
            Shape::Torus { major: 0.6, minor: 0.2 },
            Shape::Capsule { r: 0.25, half_h: 0.5 },
        ];
        let mut r = Rng::new(11, 3);
        for shape in &shapes {
            let (mut hits, mut misses, mut worst) = (0, 0, 0.0f32);
            let mut inside_rays = 0;
            let mut outside_hits = 0;
            let mut normals_checked = 0;
            for i in 0..600 {
                // half the rays start outside aiming near the shape, half start inside
                let inside = i % 2 == 1;
                let mut o = vec3(r.f32() - 0.5, r.f32() - 0.5, r.f32() - 0.5) * 6.0;
                if inside {
                    // rejection-sample the bounding box until the origin is really inside the solid
                    // (a fixed central cube would miss the torus entirely: its centre is a hole)
                    let b = shape.local_bounds(&[]);
                    let mut found = false;
                    for _ in 0..400 {
                        o = b.min + (b.max - b.min) * vec3(r.f32(), r.f32(), r.f32());
                        if implicit(shape, o) < -0.02 {
                            found = true;
                            break;
                        }
                    }
                    assert!(found, "{}: could not find an interior point", shape.kind());
                    inside_rays += 1;
                } else if implicit(shape, o) < 0.05 {
                    o = o.normalize() * 3.0;
                }
                let target = vec3(r.f32() - 0.5, r.f32() - 0.5, r.f32() - 0.5) * 1.6;
                let d = (target - o).normalize() * (0.5 + r.f32() * 2.0); // deliberately not unit length
                let got = intersect(shape, &[], &[], o, d, 1e-5, 1e30).map(|h| h.t);
                let want = reference_hit(shape, o, d, 12.0);
                if inside {
                    // a ray that starts inside a closed solid must always leave through its surface
                    assert!(got.is_some(), "{}: ray from interior point {o:?} found no exit", shape.kind());
                } else if got.is_some() {
                    outside_hits += 1;
                }
                match (got, want) {
                    (Some(a), Some(b)) => {
                        hits += 1;
                        worst = worst.max((a - b).abs() * d.length());
                        // the hit point must lie on the surface and the normal must face the ray origin side
                        let p = o + d * a;
                        assert!(implicit(shape, p).abs() < 2e-4, "{}: point off surface by {}", shape.kind(), implicit(shape, p));
                        let (ng, _) = normals(shape, &[], p, &intersect(shape, &[], &[], o, d, 1e-5, 1e30).unwrap());
                        assert!((ng.length() - 1.0).abs() < 1e-4);
                        let facing = ng.dot(d) < 0.0;
                        assert_eq!(facing, !inside, "{}: normal orientation (inside={inside})", shape.kind());
                        // The normal must be the outward gradient of the implicit function. (Checking only
                        // that it faces the ray is not enough: a box that reports the wrong FACE passes that.)
                        let grad = |h: f32| {
                            vec3(
                                implicit(shape, p + Vec3::X * h) - implicit(shape, p - Vec3::X * h),
                                implicit(shape, p + Vec3::Y * h) - implicit(shape, p - Vec3::Y * h),
                                implicit(shape, p + Vec3::Z * h) - implicit(shape, p - Vec3::Z * h),
                            )
                            .normalize()
                        };
                        let (g1, g2) = (grad(2e-3), grad(4e-4));
                        // near an edge or rim the gradient is discontinuous and the two estimates disagree: skip those
                        if g1.dot(g2) > 0.9995 {
                            normals_checked += 1;
                            assert!(ng.dot(g2) > 0.995, "{}: normal {ng:?} but surface gradient {g2:?} at {p:?} (prim {})", shape.kind(), intersect(shape, &[], &[], o, d, 1e-5, 1e30).unwrap().prim);
                        }
                    }
                    (None, None) => misses += 1,
                    // grazing rays can legitimately differ between a 40k-step reference and the exact solution
                    (Some(a), None) => {
                        let p = o + d * a;
                        assert!(implicit(shape, p).abs() < 2e-4, "{}: spurious hit", shape.kind());
                    }
                    (None, Some(b)) => {
                        // tolerated only if the reference crossing is a near-tangent graze
                        let p = o + d * (b + 2e-3 / d.length());
                        assert!(implicit(shape, p).abs() < 3e-3, "{}: missed a solid hit at t={b}", shape.kind());
                    }
                }
            }
            println!("{:9} hits {hits:3} misses {misses:3} | inside rays {inside_rays} (all exit) | outside rays hitting {outside_hits:3} | normals verified {normals_checked:3} | worst distance error {worst:.2e}", shape.kind());
            assert_eq!(inside_rays, 300);
            assert!(normals_checked >= 300, "{}: only {normals_checked} normals could be checked", shape.kind());
            assert!(outside_hits >= 50 && misses >= 50, "{}: need both hits and misses from outside to be a meaningful test", shape.kind());
            assert!(worst < 3e-4, "{}: worst error {worst}", shape.kind());
        }
    }

    #[test]
    fn box_reports_the_face_that_was_hit() {
        // Regression: a ray travelling straight up into the underside of a thin slab used to be
        // given an X-face normal, because the entry face was chosen by largest ABSOLUTE slab distance.
        let slab = Shape::Box { half: vec3(1.0, 0.01, 1.0), round: 0.0 };
        let cases = [
            (vec3(0.0, -1.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, -1.0, 0.0)),
            (vec3(0.0, -1.0, 0.0), vec3(0.02, 1.0, 0.01), vec3(0.0, -1.0, 0.0)),
            (vec3(0.3, 2.0, -0.2), vec3(0.1, -1.0, 0.05), vec3(0.0, 1.0, 0.0)),
            (vec3(-3.0, 0.001, 0.2), vec3(1.0, 0.0, 0.0), vec3(-1.0, 0.0, 0.0)),
            (vec3(0.2, 0.002, 5.0), vec3(0.0, 0.0, -2.0), vec3(0.0, 0.0, 1.0)),
            // from inside: exits through the top face
            (vec3(0.1, 0.0, 0.1), vec3(0.3, 1.0, 0.2), vec3(0.0, 1.0, 0.0)),
            (vec3(0.1, 0.0, 0.1), vec3(1.0, 0.001, 0.0), vec3(1.0, 0.0, 0.0)),
        ];
        for (o, d, want) in cases {
            let h = intersect(&slab, &[], &[], o, d, 1e-6, 1e30).unwrap();
            let (ng, ns) = normals(&slab, &[], o + d * h.t, &h);
            assert_eq!(ng, want, "ray {o:?} -> {d:?}");
            assert_eq!(ns, want);
        }
    }

    #[test]
    fn flat_shapes_and_mesh() {
        let o = vec3(0.2, 2.0, -0.1);
        let d = vec3(0.0, -1.0, 0.0);
        for s in [Shape::Disk { r: 0.5 }, Shape::Rect { hx: 0.5, hz: 0.5 }, Shape::Plane] {
            assert_eq!(intersect(&s, &[], &[], o, d, 1e-4, 1e30).unwrap().t, 2.0);
            assert!(intersect(&s, &[], &[], o, -d, 1e-4, 1e30).is_none());
        }
        assert!(intersect(&Shape::Disk { r: 0.1 }, &[], &[], o, d, 1e-4, 1e30).is_none());

        // a unit quad made of two triangles, compared against the analytic rect
        let mesh = Mesh {
            positions: vec![vec3(-0.5, 0.0, -0.5), vec3(0.5, 0.0, -0.5), vec3(0.5, 0.0, 0.5), vec3(-0.5, 0.0, 0.5)],
            normals: vec![],
            uvs: vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)],
            indices: vec![[0, 2, 1], [0, 3, 2]],
        };
        let rt = [MeshRt::build(&mesh)];
        let meshes = [mesh];
        let mut r = Rng::new(5, 5);
        for _ in 0..500 {
            let o = vec3(r.f32() * 2.0 - 1.0, r.f32() * 2.0 + 0.1, r.f32() * 2.0 - 1.0);
            let d = vec3(r.f32() - 0.5, -r.f32() - 0.05, r.f32() - 0.5);
            let a = intersect(&Shape::Mesh(0), &meshes, &rt, o, d, 1e-5, 1e30);
            let b = intersect(&Shape::Rect { hx: 0.5, hz: 0.5 }, &meshes, &rt, o, d, 1e-5, 1e30);
            assert_eq!(a.is_some(), b.is_some());
            if let (Some(a), Some(b)) = (a, b) {
                assert!((a.t - b.t).abs() < 1e-5);
                let p = o + d * a.t;
                let (ng, _) = normals(&Shape::Mesh(0), &meshes, p, &a);
                assert!((ng - Vec3::Y).length() < 1e-5, "winding gives +Y normal");
                let t = uv(&Shape::Mesh(0), &meshes, p, &a);
                assert!((t.x - (p.x + 0.5)).abs() < 1e-4 && (t.y - (p.z + 0.5)).abs() < 1e-4);
            }
        }
    }
}
