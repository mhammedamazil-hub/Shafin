//! `vl` - command-line front end.
//!
//!   vl check   scene.vl [--json]               compile only: diagnostics and the object list
//!   vl inspect scene.vl [--json]               compile + fast numeric description of the picture
//!   vl render  scene.vl -o out.png [options]   compile, inspect and render
//!
//! Exit status: 0 = success (warnings allowed), 1 = the program has errors, 2 = usage or I/O error.
//! With --json a single JSON object is written to stdout and nothing else.
#![forbid(unsafe_code)]

use std::io::Read;
use std::process::ExitCode;
use vl_core::diag;
use vl_core::json::Json;
use vl_render::inspect;
use vl_render::pipeline::{render_scene, Options};

const USAGE: &str = "usage:
  vl check   FILE [--json]
  vl inspect FILE [--json] [--threads N]
  vl render  FILE -o OUT.png [--width W] [--height H] [--samples N] [--threads N] [--json] [--quiet] [--no-inspect] [--no-denoise]
  vl version
FILE may be '-' to read the program from standard input.";

struct Args {
    cmd: String,
    file: Option<String>,
    out: Option<String>,
    json: bool,
    quiet: bool,
    no_inspect: bool,
    no_denoise: bool,
    width: Option<u32>,
    height: Option<u32>,
    samples: Option<u32>,
    threads: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or_else(|| USAGE.to_string())?;
    let mut a = Args { cmd, file: None, out: None, json: false, quiet: false, no_inspect: false, no_denoise: false, width: None, height: None, samples: None, threads: vl_core::par::available_threads() };
    let num = |name: &str, v: Option<String>, lo: u32, hi: u32| -> Result<u32, String> {
        let v = v.ok_or_else(|| format!("{name} needs a value"))?;
        match v.parse::<u32>() {
            Ok(n) if n >= lo && n <= hi => Ok(n),
            _ => Err(format!("{name} must be a whole number between {lo} and {hi}, not '{v}'")),
        }
    };
    while let Some(x) = it.next() {
        match x.as_str() {
            "--json" => a.json = true,
            "--quiet" | "-q" => a.quiet = true,
            "--no-inspect" => a.no_inspect = true,
            "--no-denoise" => a.no_denoise = true,
            "-o" | "--output" => a.out = Some(it.next().ok_or("-o needs a file name")?),
            "--width" => a.width = Some(num("--width", it.next(), 1, 16384)?),
            "--height" => a.height = Some(num("--height", it.next(), 1, 16384)?),
            "--samples" => a.samples = Some(num("--samples", it.next(), 1, 65536)?),
            "--threads" => a.threads = num("--threads", it.next(), 1, 1024)? as usize,
            s if s.starts_with('-') && s != "-" => return Err(format!("unknown option '{s}'\n{USAGE}")),
            s => {
                if a.file.is_some() {
                    return Err(format!("unexpected extra argument '{s}'\n{USAGE}"));
                }
                a.file = Some(s.to_string());
            }
        }
    }
    Ok(a)
}

fn read_source(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| format!("cannot read standard input: {e}"))?;
        return Ok(s);
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot open '{path}': {e}"))?;
    if meta.len() > vl_lang::MAX_SOURCE_BYTES as u64 + 1 {
        return Err(format!("'{path}' is {} bytes; programs are limited to {} bytes", meta.len(), vl_lang::MAX_SOURCE_BYTES));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read '{path}': {e}"))?;
    String::from_utf8(bytes).map_err(|_| format!("'{path}' is not valid UTF-8 text"))
}

fn objects_json(c: &vl_lang::Compiled) -> Json {
    let mut arr = Json::arr();
    for o in &c.objects {
        let (c0, s) = (o.bounds.center(), o.bounds.size());
        let mut j = Json::obj().with("name", o.name.as_str()).with("kind", o.kind.as_str()).with("line", o.line as f64);
        if !o.bounds.is_empty() && s.x < 1.0e4 {
            j.set("center", Json::nums(&[c0.x as f64, c0.y as f64, c0.z as f64], 3));
            j.set("size", Json::nums(&[s.x as f64, s.y as f64, s.z as f64], 3));
        }
        if let Some(m) = &o.material {
            j.set("material", m.as_str());
        }
        if let Some(p) = &o.parent {
            j.set("part_of", p.as_str());
        }
        if o.is_light {
            j.set("is_light", true);
        }
        arr.push(j);
    }
    arr
}

fn run() -> Result<ExitCode, String> {
    let a = parse_args()?;
    match a.cmd.as_str() {
        "version" | "--version" | "-V" => {
            println!("vl {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::SUCCESS);
        }
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            return Ok(ExitCode::SUCCESS);
        }
        "check" | "inspect" | "render" => {}
        other => return Err(format!("unknown command '{other}'\n{USAGE}")),
    }
    let file = a.file.clone().ok_or_else(|| format!("no input file\n{USAGE}"))?;
    if a.cmd == "render" && a.out.is_none() {
        return Err("render needs an output file:  vl render scene.vl -o out.png".to_string());
    }
    let text = read_source(&file)?;
    let mut c = vl_lang::compile(&file, &text);
    let mut out = Json::obj().with("ok", c.ok()).with("errors", c.error_count() as f64).with("warnings", c.warning_count() as f64).with("diagnostics", diag::to_json(&c.diags, &c.source));
    if !a.json && !c.diags.is_empty() {
        eprint!("{}", diag::render(&c.diags, &c.source));
    }
    if a.cmd == "check" || !c.ok() {
        out.set("objects", objects_json(&c));
        if a.json {
            println!("{}", out.to_pretty());
        } else if c.ok() {
            println!("ok: {} objects, {} warnings", c.scene.as_ref().map(|s| s.objects.len()).unwrap_or(0), c.warning_count());
        } else {
            println!("failed: {} errors, {} warnings", c.error_count(), c.warning_count());
        }
        return Ok(if c.ok() { ExitCode::SUCCESS } else { ExitCode::from(1) });
    }

    let scene = c.scene.as_mut().unwrap();
    // command-line overrides; giving only one of width/height keeps the picture's shape
    let (w0, h0) = (scene.film.width, scene.film.height);
    match (a.width, a.height) {
        (Some(w), Some(h)) => (scene.film.width, scene.film.height) = (w, h),
        (Some(w), None) => (scene.film.width, scene.film.height) = (w, ((w as f64 * h0 as f64 / w0 as f64).round() as u32).max(1)),
        (None, Some(h)) => (scene.film.width, scene.film.height) = (((h as f64 * w0 as f64 / h0 as f64).round() as u32).max(1), h),
        (None, None) => {}
    }
    if let Some(s) = a.samples {
        scene.film.spp = s;
        scene.film.min_spp = scene.film.min_spp.min(s);
    }
    if a.no_denoise {
        scene.post.denoise = false;
    }
    scene.validate().map_err(|e| format!("internal error: scene became invalid after overrides: {}", e.join("; ")))?;
    let scene = c.scene.as_ref().unwrap();

    if !(a.cmd == "render" && a.no_inspect) {
        let ins = inspect::inspect(scene, a.threads);
        if a.json {
            out.set("inspection", ins.to_json());
        } else {
            print!("{}", ins.to_text());
        }
    }
    if a.cmd == "render" {
        let path = a.out.clone().unwrap();
        let show = !a.quiet && !a.json;
        let last = std::sync::atomic::AtomicU32::new(0);
        let progress = |f: f32| {
            let pct = (f * 20.0) as u32;
            if show && last.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
                eprint!("\rrendering {}%", pct * 5);
            }
        };
        let o = render_scene(scene, &Options { threads: a.threads, progress: Some(&progress) });
        if show {
            eprintln!();
        }
        let source_hash = vl_image::image::fnv1a64(text.as_bytes());
        let opts = vl_image::png::PngOptions {
            threads: a.threads,
            text: vec![
                ("Software".to_string(), format!("vl {}", env!("CARGO_PKG_VERSION"))),
                ("vl:source-fnv1a64".to_string(), format!("{source_hash:016x}")),
                ("vl:seed".to_string(), scene.film.seed.to_string()),
                ("vl:samples".to_string(), scene.film.spp.to_string()),
                ("vl:exposure-ev".to_string(), format!("{:.3}", o.ev)),
            ],
            ..vl_image::png::PngOptions::default()
        };
        let png = vl_image::png::encode_png(&o.image, false, &opts);
        if let Some(dir) = std::path::Path::new(&path).parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
            }
        }
        std::fs::write(&path, &png).map_err(|e| format!("cannot write '{path}': {e}"))?;
        let mut r = Json::obj()
            .with("output", path.as_str())
            .with("width", scene.film.width as f64)
            .with("height", scene.film.height as f64)
            .with("samples", scene.film.spp as f64)
            .with("seconds", Json::round(o.seconds, 2))
            .with("million_rays", Json::round(o.frame.rays as f64 / 1e6, 2))
            .with("exposure_ev", Json::round(o.ev as f64, 2))
            .with("png_bytes", png.len() as f64)
            .with("image_hash", format!("{:016x}", o.image.content_hash()));
        if !o.warnings.is_empty() {
            r.set("renderer_warnings", o.warnings.clone());
        }
        if a.json {
            out.set("render", r);
        } else {
            for w in &o.warnings {
                eprintln!("renderer warning: {w}");
            }
            println!("wrote {path}: {}x{}, {} samples, {:.1} s, EV {:.2}", scene.film.width, scene.film.height, scene.film.spp, o.seconds, o.ev);
        }
    }
    if a.json {
        println!("{}", out.to_pretty());
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("vl: {msg}");
            ExitCode::from(2)
        }
    }
}
