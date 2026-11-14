//! Numeric inspection: what a text-only author needs to know about the picture
//! without looking at it.
//!
//! Everything here is measured from the renderer's own buffers at a small
//! resolution: which object each pixel shows, how much of each object's
//! silhouette is hidden and by what, where it sits in the frame, what colour it
//! appears, and how the image is exposed. Findings that a person would call a
//! problem are reported as warnings with a concrete suggestion.
use crate::geom;
use crate::integrator::{self, FilmOverride, RenderOpts};
use crate::pipeline;
use crate::post;
use crate::rt::SceneRt;
use vl_core::color;
use vl_core::json::Json;
use vl_core::math::{Aabb, Vec3};
use vl_ir::scene::{Scene, Shape};

/// Longest side, in pixels, of the buffers used for inspection.
pub const INSPECT_SIDE: u32 = 256;

#[derive(Clone, Debug)]
pub struct ObjectReport {
    pub name: String,
    pub kind: &'static str,
    /// Percentage of the image this object's visible pixels cover.
    pub image_pct: f32,
    /// Visible pixels divided by the pixels its silhouette would cover with nothing in front
    /// (`None` for an infinite plane or an object entirely outside the frame).
    pub visible_fraction: Option<f32>,
    /// Bounding box of the visible part in the frame, 0..1 from the top-left corner.
    pub frame_box: Option<[f32; 4]>,
    pub touches_frame_edge: bool,
    pub distance: f32,
    /// Mean colour as it appears in the picture (display-linear), where visible.
    pub seen_color: Option<Vec3>,
    /// Names of the objects that hide the most of it, most first.
    pub hidden_by: Vec<String>,
    pub in_frame: bool,
    pub is_light: bool,
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub code: &'static str,
    pub object: Option<String>,
    pub message: String,
    pub hint: String,
}

pub struct Inspection {
    pub width: u32,
    pub height: u32,
    pub ev: f32,
    pub auto_exposure: bool,
    /// Mean display luminance, 0 (black) .. 1 (white), after tone mapping.
    pub mean_brightness: f32,
    pub dark_pct: f32,
    pub bright_pct: f32,
    pub background_pct: f32,
    pub objects: Vec<ObjectReport>,
    pub findings: Vec<Finding>,
    pub map: Vec<String>,
    pub legend: Vec<(char, String)>,
}

fn top_level(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

pub fn inspect(scene: &Scene, threads: usize) -> Inspection {
    let rt = SceneRt::build(scene);
    let f = &scene.film;
    let k = (INSPECT_SIDE as f32 / f.width.max(f.height) as f32).min(1.0);
    let (w, h) = (((f.width as f32 * k).round() as usize).max(2), ((f.height as f32 * k).round() as usize).max(2));
    let over = FilmOverride { width: Some(w as u32), height: Some(h as u32), spp: Some(24), max_depth: Some(f.max_depth.min(6)) };

    // 1. which objects does each pixel show (pinhole, crisp): the first surface and, when
    //    that is glass or water, what is seen through it. Ice in a glass is not "hidden".
    const LAYERS: usize = 4;
    let layers: Vec<[u32; LAYERS]> = {
        let rows: Vec<Vec<[u32; LAYERS]>> = vl_core::par::par_map(h, threads.max(1), |y| {
            (0..w)
                .map(|x| {
                    let (o, d) = rt.cam.pinhole_ray(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32);
                    let mut out = [u32::MAX; LAYERS];
                    let (mut n, mut t0) = (0usize, 0.0f32);
                    for _ in 0..24 {
                        let Some(hit) = rt.closest(o, d, t0, f32::MAX, true) else { break };
                        if !out[..n].contains(&hit.obj) {
                            out[n] = hit.obj;
                            n += 1;
                        }
                        if n == LAYERS || !rt.mats[rt.insts[hit.obj as usize].material as usize].transmissive {
                            break;
                        }
                        t0 = hit.t * (1.0 + 1e-4) + 1e-5;
                    }
                    out
                })
                .collect()
        });
        rows.into_iter().flatten().collect()
    };
    let ids: Vec<u32> = layers.iter().map(|l| l[0]).collect();

    // 2. a small beauty render for colour and exposure statistics
    let (ev, auto_exposure) = pipeline::measure_exposure(&rt, threads);
    let clamp = if f.clamp_indirect > 0.0 { f.clamp_indirect / post::exposure_scale(ev) } else { 0.0 };
    let frame = integrator::render(&rt, &RenderOpts { threads, clamp, film: over, ..RenderOpts::default() });
    let img = post::develop(&frame, &frame.color, &scene.post, ev);
    let px = |i: usize| Vec3 { x: img.data[i * 4], y: img.data[i * 4 + 1], z: img.data[i * 4 + 2] };
    let n = w * h;
    let (mut sum_l, mut dark, mut bright, mut background) = (0.0f64, 0usize, 0usize, 0usize);
    for i in 0..n {
        let l = color::luminance(px(i));
        sum_l += l as f64;
        dark += (l < 0.01) as usize;
        bright += (l > 0.95) as usize;
        background += (ids[i] == u32::MAX) as usize;
    }

    // 3. per-object visibility
    let nobj = scene.objects.len();
    let mut visible = vec![0usize; nobj];
    let mut boxes = vec![[usize::MAX, usize::MAX, 0usize, 0usize]; nobj];
    let mut colour = vec![Vec3::ZERO; nobj];
    for y in 0..h {
        for x in 0..w {
            for &id in layers[y * w + x].iter().take_while(|&&id| id != u32::MAX) {
                let i = id as usize;
                visible[i] += 1;
                let b = &mut boxes[i];
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
                colour[i] += px(y * w + x);
            }
        }
    }
    let mut objects = Vec::with_capacity(nobj);
    for (i, o) in scene.objects.iter().enumerate() {
        let inst = &rt.insts[i];
        let is_light = rt.mats[inst.material as usize].emissive;
        let wb = o.world_bounds(&scene.meshes);
        let distance = (wb.center() - scene.camera.pos).length();
        let plane = matches!(o.shape, Shape::Plane);
        // pixel rectangle that can contain the object
        let mut rect = [w as f32, h as f32, 0.0f32, 0.0f32];
        let mut behind = false;
        for c in 0..8 {
            let p = Vec3 { x: if c & 1 == 0 { wb.min.x } else { wb.max.x }, y: if c & 2 == 0 { wb.min.y } else { wb.max.y }, z: if c & 4 == 0 { wb.min.z } else { wb.max.z } };
            match rt.cam.project(p, w as f32, h as f32) {
                Some((x, y, _)) => {
                    rect = [rect[0].min(x), rect[1].min(y), rect[2].max(x), rect[3].max(y)];
                }
                None => behind = true,
            }
        }
        let (x0, y0, x1, y1) = if behind || plane {
            (0usize, 0usize, w, h)
        } else {
            ((rect[0].floor().max(0.0) as usize).min(w), (rect[1].floor().max(0.0) as usize).min(h), (rect[2].ceil().max(0.0) as usize + 1).min(w), (rect[3].ceil().max(0.0) as usize + 1).min(h))
        };
        let (mut silhouette, mut edge) = (0usize, false);
        let mut blockers: Vec<(u32, usize)> = Vec::new();
        if !plane && !o.hidden {
            for y in y0..y1 {
                for x in x0..x1 {
                    let (ro, rd) = rt.cam.pinhole_ray(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32);
                    let (lo, ld) = (inst.to_obj.point(ro), inst.to_obj.vector(rd));
                    if geom::intersect(&inst.shape, &scene.meshes, &rt.mesh_rt, lo, ld, 0.0, f32::MAX).is_some() {
                        silhouette += 1;
                        edge |= x == 0 || y == 0 || x == w - 1 || y == h - 1;
                        // what stopped the view here, if this object is not among the things seen
                        let l = &layers[y * w + x];
                        let seen = l.iter().rev().copied().find(|&id| id != u32::MAX).unwrap_or(u32::MAX);
                        if !l.contains(&(i as u32)) && seen != u32::MAX {
                            match blockers.iter_mut().find(|b| b.0 == seen) {
                                Some(b) => b.1 += 1,
                                None => blockers.push((seen, 1)),
                            }
                        }
                    }
                }
            }
        }
        blockers.sort_by(|a, b| b.1.cmp(&a.1));
        let hidden_by: Vec<String> = blockers.iter().take(3).filter(|b| b.1 * 20 >= silhouette.max(1)).map(|b| scene.objects[b.0 as usize].name.clone()).collect();
        let v = visible[i];
        objects.push(ObjectReport {
            name: o.name.clone(),
            kind: o.shape.kind(),
            image_pct: 100.0 * v as f32 / n as f32,
            visible_fraction: if plane || silhouette == 0 { None } else { Some((v as f32 / silhouette as f32).min(1.0)) },
            frame_box: if v > 0 { Some([boxes[i][0] as f32 / w as f32, boxes[i][1] as f32 / h as f32, (boxes[i][2] + 1) as f32 / w as f32, (boxes[i][3] + 1) as f32 / h as f32]) } else { None },
            touches_frame_edge: edge,
            distance,
            seen_color: if v > 0 { Some(colour[i] * (1.0 / v as f32)) } else { None },
            hidden_by,
            in_frame: plane || silhouette > 0,
            is_light,
        });
    }

    // 4. findings
    let mut findings = Vec::new();
    let full_res_px = |pct: f32| pct / 100.0 * (f.width as f32 * f.height as f32);
    let subject_px = ids.iter().filter(|&&id| id != u32::MAX && !matches!(scene.objects[id as usize].shape, Shape::Plane)).count();
    let subject_pct = 100.0 * subject_px as f32 / n as f32;
    // Findings are about whole things, not their parts: a table whose top hides two of its
    // legs is a normal table. Parts of one group (names sharing the text before the first
    // dot) are added up, and hiding one another does not count.
    struct Whole {
        name: String,
        image_pct: f32,
        visible: f32,
        silhouette: f32,
        in_frame: bool,
        edge: bool,
        is_light: bool,
        hidden_by: Vec<String>,
        bounds: Aabb,
        /// Bounds of the hollow vessels among its parts (a bowl, a mug).
        cups: Vec<Aabb>,
    }
    let mut wholes: Vec<Whole> = Vec::new();
    for (o, so) in objects.iter().zip(&scene.objects) {
        if matches!(so.shape, Shape::Plane) || so.hidden {
            continue;
        }
        let top = top_level(&o.name).to_string();
        let i = match wholes.iter().position(|w| w.name == top) {
            Some(i) => i,
            None => {
                wholes.push(Whole { name: top.clone(), image_pct: 0.0, visible: 0.0, silhouette: 0.0, in_frame: false, edge: false, is_light: true, hidden_by: Vec::new(), bounds: Aabb::EMPTY, cups: Vec::new() });
                wholes.len() - 1
            }
        };
        let w = &mut wholes[i];
        let wb = so.shape.local_bounds(&scene.meshes).transformed(&so.xform);
        w.bounds = w.bounds.union(&wb);
        if matches!(so.shape, Shape::Cup { .. }) {
            w.cups.push(wb);
        }
        w.image_pct += o.image_pct;
        w.in_frame |= o.in_frame;
        w.edge |= o.touches_frame_edge;
        w.is_light &= o.is_light;
        if let Some(vf) = o.visible_fraction {
            // image_pct is the visible share; the silhouette follows from the visible fraction
            w.visible += o.image_pct;
            w.silhouette += if vf > 0.0 { o.image_pct / vf } else { 0.0 };
        }
        for h in &o.hidden_by {
            if top_level(h) != top && !w.hidden_by.contains(&top_level(h).to_string()) {
                w.hidden_by.push(top_level(h).to_string());
            }
        }
    }
    // Things stacked, piled, put inside one another or built into one another hide each other
    // by design: the orange at the bottom of a bowl of oranges, the far wheels of a car made
    // of boxes. Only what is hidden by something that merely stands in front of it is reported.
    let overlap = |a: &Aabb, b: &Aabb| Vec3 { x: a.max.x.min(b.max.x) - a.min.x.max(b.min.x), y: a.max.y.min(b.max.y) - a.min.y.max(b.min.y), z: a.max.z.min(b.max.z) - a.min.z.max(b.min.z) };
    // one above the other, touching or nearly
    let stacked = |a: &Aabb, b: &Aabb| {
        let o = overlap(a, b);
        o.x > 0.1 * a.size().x.min(b.size().x) && o.z > 0.1 * a.size().z.min(b.size().z) && o.y > -0.05
    };
    // `a` is built into `b`: a good part of it lies inside b's bounds
    let built_into = |a: &Aabb, b: &Aabb| {
        let (o, sa) = (overlap(a, b), a.size());
        o.x > 0.0 && o.y > 0.0 && o.z > 0.0 && o.x * o.y * o.z >= 0.15 * (sa.x * sa.y * sa.z).max(1e-12)
    };
    // the vessel a thing lies in: its middle is within the vessel's outline and it reaches below the rim
    let vessel_of = |w: &Whole| -> Option<(usize, usize)> {
        let c = w.bounds.center();
        for (i, o) in wholes.iter().enumerate() {
            if o.name == w.name {
                continue;
            }
            for (j, cup) in o.cups.iter().enumerate() {
                if c.x > cup.min.x && c.x < cup.max.x && c.z > cup.min.z && c.z < cup.max.z && w.bounds.min.y < cup.max.y && w.bounds.max.y > cup.min.y {
                    return Some((i, j));
                }
            }
        }
        None
    };
    let vessels: Vec<Option<(usize, usize)>> = wholes.iter().map(vessel_of).collect();
    let together = |a: usize, b: usize| {
        let (ba, bb) = (&wholes[a].bounds, &wholes[b].bounds);
        stacked(ba, bb) || built_into(ba, bb) || built_into(bb, ba) || (vessels[a].is_some() && vessels[a] == vessels[b]) || vessels[a].map(|v| v.0) == Some(b) || vessels[b].map(|v| v.0) == Some(a)
    };
    // ... or with the thing `a` is built into: the cabin on the car body hides the far wheel
    let belongs = |a: usize, h: usize| together(a, h) || (0..wholes.len()).any(|c| c != a && c != h && built_into(&wholes[a].bounds, &wholes[c].bounds) && together(c, h));
    let in_front: Vec<Vec<String>> = (0..wholes.len()).map(|a| wholes[a].hidden_by.iter().filter(|h| wholes.iter().position(|o| &o.name == *h).map(|b| !belongs(a, b)).unwrap_or(true)).cloned().collect()).collect();
    for (w, in_front) in wholes.iter().zip(&in_front) {
        let name = Some(w.name.clone());
        // A lamp that lights the scene from outside the picture, or from behind something,
        // is how lamps are normally used. Reporting it sent a model off to "fix" a good scene.
        if w.is_light && (!w.in_frame || w.image_pct == 0.0) {
            continue;
        }
        if !w.in_frame {
            findings.push(Finding { code: "V101", object: name, message: format!("'{}' is outside the picture: the camera does not see it at all", w.name), hint: format!("point the camera at it ( look_at: {} ), move it into view, or widen 'fov'", w.name) });
        } else if w.image_pct == 0.0 && (!in_front.is_empty() || w.hidden_by.is_empty()) {
            let by = if w.hidden_by.is_empty() { "other objects".to_string() } else { format!("'{}'", w.hidden_by.join("', '")) };
            findings.push(Finding { code: "V102", object: name, message: format!("'{}' is completely hidden behind {by}", w.name), hint: "move it, move what hides it, or choose another camera position ('from', or 'azimuth' / 'elevation')".to_string() });
        } else if w.image_pct > 0.0 {
            let vf = if w.silhouette > 0.0 { (w.visible / w.silhouette).min(1.0) } else { 1.0 };
            if vf < 0.35 && !in_front.is_empty() {
                findings.push(Finding {
                    code: "V103",
                    object: name.clone(),
                    message: format!("'{}' is {:.0}% hidden behind '{}'", w.name, (1.0 - vf) * 100.0, w.hidden_by.join("', '")),
                    hint: "move it, move what hides it, or choose another camera position".to_string(),
                });
            }
            // small because it is small or far away - not because most of it is behind something
            let size_pct = w.silhouette.max(w.image_pct);
            if full_res_px(size_pct) < 150.0 && !w.is_light {
                findings.push(Finding {
                    code: "V104",
                    object: name.clone(),
                    message: format!("'{}' is tiny in the picture (about {:.0} pixels of the final image)", w.name, full_res_px(size_pct)),
                    hint: "move the camera closer ( distance: ... ), narrow 'fov', or make the object larger".to_string(),
                });
            }
            // What other things stand on (a desk under a still life, a road under a car) is the
            // stage; it normally runs out of the picture.
            let stage = wholes.iter().any(|o| {
                o.name != w.name && (o.bounds.min.y - w.bounds.max.y).abs() < 0.02 + 0.01 * w.bounds.size().y && o.bounds.min.x < w.bounds.max.x && o.bounds.max.x > w.bounds.min.x && o.bounds.min.z < w.bounds.max.z && o.bounds.max.z > w.bounds.min.z
            });
            if w.edge && w.image_pct < 35.0 && !w.is_light && !stage {
                findings.push(Finding { code: "V105", object: name, message: format!("'{}' is cut off by the edge of the picture", w.name), hint: "move the camera back ( distance: ... ), widen 'fov', or aim the camera with 'look_at'".to_string() });
            }
        }
    }
    let mean = (sum_l / n as f64) as f32;
    let (dark_pct, bright_pct) = (100.0 * dark as f32 / n as f32, 100.0 * bright as f32 / n as f32);
    if nobj > 0 && subject_pct < 3.0 && scene.objects.iter().any(|o| !matches!(o.shape, Shape::Plane)) {
        findings.push(Finding { code: "V110", object: None, message: format!("the objects fill only {subject_pct:.1}% of the picture; it is mostly empty"), hint: "omit 'from' so the camera frames the scene automatically, or reduce 'distance' / 'fov'".to_string() });
    }
    if mean < 0.04 {
        let hint = if auto_exposure { "the scene has almost no light: add  sky { }  or a  light" } else { "lower 'exposure' (each step doubles the brightness) or write  exposure: auto" };
        findings.push(Finding { code: "V111", object: None, message: format!("the picture is almost black (mean brightness {:.0}%)", mean * 100.0), hint: hint.to_string() });
    } else if bright_pct > 40.0 {
        findings.push(Finding { code: "V112", object: None, message: format!("{bright_pct:.0}% of the picture is blown out to white"), hint: "raise 'exposure' (each step halves the brightness), write  exposure: auto, or reduce the lights".to_string() });
    }

    // 5. a small text map of what is where
    let cols = 48usize.min(w);
    let rows = ((cols as f32 * h as f32 / w as f32) * 0.5).round().max(1.0) as usize;
    let mut tops: Vec<String> = Vec::new();
    for o in &scene.objects {
        let t = top_level(&o.name).to_string();
        if !tops.contains(&t) {
            tops.push(t);
        }
    }
    let letters: Vec<char> = ('A'..='Z').chain('a'..='z').collect();
    let mut map = Vec::with_capacity(rows);
    let mut used = vec![false; tops.len()];
    for r in 0..rows {
        let mut line = String::with_capacity(cols);
        for c in 0..cols {
            let (xa, xb) = (c * w / cols, ((c + 1) * w / cols).max(c * w / cols + 1));
            let (ya, yb) = (r * h / rows, ((r + 1) * h / rows).max(r * h / rows + 1));
            let mut counts: Vec<(usize, usize)> = Vec::new();
            let mut bg = 0;
            for y in ya..yb.min(h) {
                for x in xa..xb.min(w) {
                    let id = ids[y * w + x];
                    if id == u32::MAX {
                        bg += 1;
                        continue;
                    }
                    let t = tops.iter().position(|t| t == top_level(&scene.objects[id as usize].name)).unwrap();
                    match counts.iter_mut().find(|e| e.0 == t) {
                        Some(e) => e.1 += 1,
                        None => counts.push((t, 1)),
                    }
                }
            }
            counts.sort_by(|a, b| b.1.cmp(&a.1));
            line.push(match counts.first() {
                Some(&(t, cnt)) if cnt >= bg => {
                    used[t] = true;
                    if matches!(scene.objects.iter().find(|o| top_level(&o.name) == tops[t]).map(|o| &o.shape), Some(Shape::Plane)) {
                        ':'
                    } else {
                        letters.get(t).copied().unwrap_or('#')
                    }
                }
                _ => '.',
            });
        }
        map.push(line);
    }
    let legend: Vec<(char, String)> = tops
        .iter()
        .enumerate()
        .filter(|(t, _)| used[*t])
        .map(|(t, name)| {
            let plane = matches!(scene.objects.iter().find(|o| top_level(&o.name) == name).map(|o| &o.shape), Some(Shape::Plane));
            (if plane { ':' } else { letters.get(t).copied().unwrap_or('#') }, name.clone())
        })
        .collect();

    Inspection { width: f.width, height: f.height, ev, auto_exposure, mean_brightness: mean, dark_pct, bright_pct, background_pct: 100.0 * background as f32 / n as f32, objects, findings, map, legend }
}

impl Inspection {
    pub fn to_json(&self) -> Json {
        let r = |v: f32, d: u32| Json::round(v as f64, d);
        let mut objs = Json::arr();
        for o in &self.objects {
            let mut j = Json::obj().with("name", o.name.as_str()).with("shape", o.kind).with("percent_of_image", r(o.image_pct, 2)).with("in_frame", o.in_frame).with("distance_m", r(o.distance, 2));
            if let Some(v) = o.visible_fraction {
                j.set("visible_percent_of_itself", r(v * 100.0, 0));
            }
            if let Some(b) = o.frame_box {
                j.set("frame_box_left_top_right_bottom", Json::nums(&[b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64], 2));
                j.set("cut_off_by_frame", o.touches_frame_edge);
            }
            if let Some(c) = o.seen_color {
                j.set("appears", color::describe(c));
                j.set("appears_hex", color::hex_string(c));
            }
            if !o.hidden_by.is_empty() {
                j.set("partly_hidden_by", o.hidden_by.clone());
            }
            if o.is_light {
                j.set("is_light", true);
            }
            objs.push(j);
        }
        let mut findings = Json::arr();
        for f in &self.findings {
            let mut j = Json::obj().with("code", f.code).with("message", f.message.as_str()).with("hint", f.hint.as_str());
            if let Some(o) = &f.object {
                j.set("object", o.as_str());
            }
            findings.push(j);
        }
        let mut legend = Json::obj();
        for (c, name) in &self.legend {
            legend.set(&c.to_string(), name.as_str());
        }
        legend.set(".", "background (sky or empty)");
        Json::obj()
            .with(
                "image",
                Json::obj()
                    .with("width", self.width as f64)
                    .with("height", self.height as f64)
                    .with("exposure_ev", r(self.ev, 2))
                    .with("exposure", if self.auto_exposure { "auto" } else { "manual" })
                    .with("mean_brightness_percent", r(self.mean_brightness * 100.0, 0))
                    .with("black_pixels_percent", r(self.dark_pct, 1))
                    .with("white_pixels_percent", r(self.bright_pct, 1))
                    .with("background_percent", r(self.background_pct, 1)),
            )
            .with("objects", objs)
            .with("layout_map", self.map.clone())
            .with("layout_legend", legend)
            .with("visual_findings", findings)
    }

    /// Compact plain-text form, cheaper in tokens than JSON for a language model.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "image {}x{}, exposure EV {:.1} ({}), mean brightness {:.0}%, {:.0}% black, {:.0}% white, {:.0}% background\n",
            self.width,
            self.height,
            self.ev,
            if self.auto_exposure { "auto" } else { "manual" },
            self.mean_brightness * 100.0,
            self.dark_pct,
            self.bright_pct,
            self.background_pct
        ));
        s.push_str("objects (share of image, how much of itself is visible, position in frame, apparent colour):\n");
        for o in &self.objects {
            let vis = match o.visible_fraction {
                Some(v) => format!("{:.0}% visible", v * 100.0),
                None if o.in_frame => "floor".to_string(),
                None => "NOT IN FRAME".to_string(),
            };
            let pos = match o.frame_box {
                Some(b) => {
                    let (cx, cy) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
                    let hx = if cx < 0.36 { "left" } else if cx > 0.64 { "right" } else { "centre" };
                    let vy = if cy < 0.36 { "top" } else if cy > 0.64 { "bottom" } else { "middle" };
                    format!("{vy}-{hx}")
                }
                None => "-".to_string(),
            };
            let col = o.seen_color.map(color::describe).unwrap_or_else(|| "-".to_string());
            let hid = if o.hidden_by.is_empty() { String::new() } else { format!(", behind {}", o.hidden_by.join("+")) };
            s.push_str(&format!("  {}: {:.1}% of image, {vis}{hid}, {pos}, {col}{}\n", o.name, o.image_pct, if o.touches_frame_edge && o.visible_fraction.is_some() { ", cut off by frame" } else { "" }));
        }
        s.push_str("layout map (top of image first):\n");
        for line in &self.map {
            s.push_str("  ");
            s.push_str(line);
            s.push('\n');
        }
        s.push_str("  legend: ");
        s.push_str(&self.legend.iter().map(|(c, n)| format!("{c}={n}")).collect::<Vec<_>>().join(" "));
        s.push_str(" .=background\n");
        if self.findings.is_empty() {
            s.push_str("visual findings: none\n");
        } else {
            s.push_str("visual findings:\n");
            for f in &self.findings {
                s.push_str(&format!("  {} {} -> {}\n", f.code, f.message, f.hint));
            }
        }
        s
    }
}
