//! Bounding volume hierarchy built with binned SAH. One implementation serves
//! the object level (instances) and the triangle level (meshes).
use vl_core::math::{vec3, Aabb, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub bounds: Aabb,
    /// Leaf: first index into `prims`. Inner node: index of the left child (right = left + 1).
    pub a: u32,
    /// Number of primitives for a leaf, 0 for an inner node.
    pub count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Bvh {
    pub nodes: Vec<Node>,
    pub prims: Vec<u32>,
}

const BINS: usize = 12;
const STACK: usize = 96;

fn area(b: &Aabb) -> f32 {
    if b.is_empty() {
        0.0
    } else {
        b.surface_area()
    }
}

impl Bvh {
    pub fn build(bounds: &[Aabb], max_leaf: usize) -> Bvh {
        let n = bounds.len();
        if n == 0 {
            return Bvh::default();
        }
        let max_leaf = max_leaf.max(1);
        let centroids: Vec<Vec3> = bounds.iter().map(|b| b.center()).collect();
        let mut prims: Vec<u32> = (0..n as u32).collect();
        let mut nodes: Vec<Node> = Vec::with_capacity(2 * n);
        nodes.push(Node { bounds: Aabb::EMPTY, a: 0, count: 0 });
        let mut stack: Vec<(usize, usize, usize)> = vec![(0, 0, n)];
        while let Some((ni, s, e)) = stack.pop() {
            let mut nb = Aabb::EMPTY;
            let mut cb = Aabb::EMPTY;
            for &p in &prims[s..e] {
                nb = nb.union(&bounds[p as usize]);
                cb.grow(centroids[p as usize]);
            }
            nodes[ni].bounds = nb;
            let cnt = e - s;
            if cnt <= max_leaf {
                nodes[ni].a = s as u32;
                nodes[ni].count = cnt as u32;
                continue;
            }
            // binned SAH over the three axes
            let ext = cb.max - cb.min;
            let mut best: Option<(usize, usize, f32)> = None; // (axis, split bin, cost)
            for axis in 0..3 {
                if !(ext[axis] > 0.0) {
                    continue;
                }
                let scale = BINS as f32 / ext[axis];
                let mut bin_b = [Aabb::EMPTY; BINS];
                let mut bin_n = [0usize; BINS];
                for &p in &prims[s..e] {
                    let b = (((centroids[p as usize][axis] - cb.min[axis]) * scale) as usize).min(BINS - 1);
                    bin_b[b] = bin_b[b].union(&bounds[p as usize]);
                    bin_n[b] += 1;
                }
                let mut right_area = [0.0f32; BINS];
                let mut acc = Aabb::EMPTY;
                let mut right_n = [0usize; BINS];
                let mut cn = 0;
                for i in (1..BINS).rev() {
                    acc = acc.union(&bin_b[i]);
                    cn += bin_n[i];
                    right_area[i] = area(&acc);
                    right_n[i] = cn;
                }
                let mut acc = Aabb::EMPTY;
                let mut ln = 0;
                for i in 1..BINS {
                    acc = acc.union(&bin_b[i - 1]);
                    ln += bin_n[i - 1];
                    if ln == 0 || right_n[i] == 0 {
                        continue;
                    }
                    let cost = area(&acc) * ln as f32 + right_area[i] * right_n[i] as f32;
                    if best.map(|b| cost < b.2).unwrap_or(true) {
                        best = Some((axis, i, cost));
                    }
                }
            }
            let leaf_cost = area(&nb) * cnt as f32;
            let mid = match best {
                Some((axis, split, cost)) if cost < leaf_cost || cnt > 4 * max_leaf => {
                    let scale = BINS as f32 / ext[axis];
                    let (mut i, mut j) = (s, e);
                    while i < j {
                        let b = (((centroids[prims[i] as usize][axis] - cb.min[axis]) * scale) as usize).min(BINS - 1);
                        if b < split {
                            i += 1;
                        } else {
                            j -= 1;
                            prims.swap(i, j);
                        }
                    }
                    i
                }
                Some(_) => {
                    // splitting does not pay and the leaf is small enough
                    nodes[ni].a = s as u32;
                    nodes[ni].count = cnt as u32;
                    continue;
                }
                None => (s + e) / 2, // all centroids coincide: split by index
            };
            let mid = if mid == s || mid == e { (s + e) / 2 } else { mid };
            let left = nodes.len();
            nodes.push(Node { bounds: Aabb::EMPTY, a: 0, count: 0 });
            nodes.push(Node { bounds: Aabb::EMPTY, a: 0, count: 0 });
            nodes[ni].a = left as u32;
            nodes[ni].count = 0;
            stack.push((left, s, mid));
            stack.push((left + 1, mid, e));
        }
        Bvh { nodes, prims }
    }

    pub fn bounds(&self) -> Aabb {
        self.nodes.first().map(|n| n.bounds).unwrap_or(Aabb::EMPTY)
    }

    /// Closest-hit traversal. `f(prim, tmax)` tests one primitive and returns the new
    /// (possibly smaller) tmax. Returns the final tmax.
    #[inline]
    pub fn closest<F: FnMut(u32, f32) -> f32>(&self, o: Vec3, d: Vec3, tmin: f32, mut tmax: f32, mut f: F) -> f32 {
        if self.nodes.is_empty() {
            return tmax;
        }
        let inv = vec3(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        if !self.nodes[0].bounds.hit(o, inv, tmin, tmax) {
            return tmax;
        }
        let mut stack = [0u32; STACK];
        let mut sp = 0usize;
        let mut ni = 0u32;
        loop {
            let node = &self.nodes[ni as usize];
            if node.count > 0 {
                for i in node.a..node.a + node.count {
                    tmax = f(self.prims[i as usize], tmax);
                }
            } else {
                let (l, r) = (node.a, node.a + 1);
                let dl = self.nodes[l as usize].bounds.hit_dist(o, inv, tmin, tmax);
                let dr = self.nodes[r as usize].bounds.hit_dist(o, inv, tmin, tmax);
                let (near, far, dn, df) = if dl <= dr { (l, r, dl, dr) } else { (r, l, dr, dl) };
                if dn < f32::INFINITY {
                    if df < f32::INFINITY && sp < STACK {
                        stack[sp] = far;
                        sp += 1;
                    }
                    ni = near;
                    continue;
                }
            }
            // pop, skipping nodes that are now behind the closest hit
            loop {
                if sp == 0 {
                    return tmax;
                }
                sp -= 1;
                ni = stack[sp];
                if self.nodes[ni as usize].bounds.hit(o, inv, tmin, tmax) {
                    break;
                }
            }
        }
    }

    /// Any-hit traversal: returns true as soon as `f(prim)` reports an occluder.
    #[inline]
    pub fn any<F: FnMut(u32) -> bool>(&self, o: Vec3, d: Vec3, tmin: f32, tmax: f32, mut f: F) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let inv = vec3(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let mut stack = [0u32; STACK];
        let mut sp = 1usize;
        while sp > 0 {
            sp -= 1;
            let node = &self.nodes[stack[sp] as usize];
            if !node.bounds.hit(o, inv, tmin, tmax) {
                continue;
            }
            if node.count > 0 {
                for i in node.a..node.a + node.count {
                    if f(self.prims[i as usize]) {
                        return true;
                    }
                }
            } else if sp + 2 <= STACK {
                stack[sp] = node.a;
                stack[sp + 1] = node.a + 1;
                sp += 2;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::rng::Rng;

    fn rand_boxes(n: usize, seed: u64) -> Vec<Aabb> {
        let mut r = Rng::new(seed, 1);
        (0..n)
            .map(|_| {
                let c = vec3(r.f32() * 20.0 - 10.0, r.f32() * 20.0 - 10.0, r.f32() * 20.0 - 10.0);
                let h = vec3(r.f32() * 0.5 + 0.01, r.f32() * 0.5 + 0.01, r.f32() * 0.5 + 0.01);
                Aabb::new(c - h, c + h)
            })
            .collect()
    }

    #[test]
    fn traversal_matches_brute_force() {
        for (n, seed) in [(1usize, 1u64), (2, 2), (7, 3), (500, 4), (5000, 5)] {
            let boxes = rand_boxes(n, seed);
            let bvh = Bvh::build(&boxes, 4);
            // every primitive appears exactly once
            let mut seen = bvh.prims.clone();
            seen.sort_unstable();
            assert_eq!(seen, (0..n as u32).collect::<Vec<_>>());
            let mut r = Rng::new(seed, 9);
            for _ in 0..400 {
                let o = vec3(r.f32() * 30.0 - 15.0, r.f32() * 30.0 - 15.0, r.f32() * 30.0 - 15.0);
                let d = vec3(r.f32() - 0.5, r.f32() - 0.5, r.f32() - 0.5).normalize();
                let inv = vec3(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
                let prim = |p: u32, tmax: f32| boxes[p as usize].hit_dist(o, inv, 0.0, tmax).min(tmax);
                let brute = (0..n as u32).fold(1e30f32, |t, p| prim(p, t));
                let got = bvh.closest(o, d, 0.0, 1e30, prim);
                assert_eq!(got, brute);
                assert_eq!(bvh.any(o, d, 0.0, 1e30, |p| boxes[p as usize].hit(o, inv, 0.0, 1e30)), brute < 1e30);
            }
        }
    }

    #[test]
    fn degenerate_inputs() {
        assert_eq!(Bvh::build(&[], 4).closest(Vec3::ZERO, Vec3::X, 0.0, 5.0, |_, t| t), 5.0);
        // many identical boxes must not recurse forever or build an unbounded tree
        let same = vec![Aabb::new(Vec3::ZERO, Vec3::ONE); 1000];
        let bvh = Bvh::build(&same, 4);
        assert!(bvh.nodes.len() < 2100);
        let mut count = 0;
        bvh.closest(vec3(0.5, 0.5, -5.0), Vec3::Z, 0.0, 100.0, |_, t| {
            count += 1;
            t
        });
        assert_eq!(count, 1000);
    }
}
