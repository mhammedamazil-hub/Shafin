//! Colour: sRGB transfer functions, parsing, colour temperature, and
//! plain-language colour naming (so a text-only model can "read" a colour).
use crate::dmath;
use crate::math::{vec3, Vec3};

#[inline]
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        dmath::pow((c + 0.055) / 1.055, 2.4)
    }
}

#[inline]
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * dmath::pow(c, 1.0 / 2.4) - 0.055
    }
}

/// Linear [0,1] -> 8-bit sRGB code value.
#[inline]
pub fn to_srgb8(c: f32) -> u8 {
    let c = if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) };
    (linear_to_srgb(c) * 255.0 + 0.5) as u8
}

#[inline]
pub fn to_srgb16(c: f32) -> u16 {
    let c = if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) };
    (linear_to_srgb(c) * 65535.0 + 0.5) as u16
}

/// 8-bit sRGB code value -> linear, via an exact 256-entry table.
pub fn srgb8_to_linear(v: u8) -> f32 {
    use std::sync::OnceLock;
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0.0f32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            *e = srgb_to_linear(i as f32 / 255.0);
        }
        t
    })[v as usize]
}

pub fn srgb_vec_to_linear(c: Vec3) -> Vec3 {
    c.map(srgb_to_linear)
}

#[inline]
pub fn luminance(c: Vec3) -> f32 {
    0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z
}

pub fn hex_string(linear: Vec3) -> String {
    format!("#{:02x}{:02x}{:02x}", to_srgb8(linear.x), to_srgb8(linear.y), to_srgb8(linear.z))
}

/// Parse `#rgb`, `#rrggbb` or `#rrggbbaa` into LINEAR rgb + alpha.
pub fn parse_hex(s: &str) -> Option<(Vec3, f32)> {
    let h = s.strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
    let (r, g, b, a) = match h.len() {
        3 => (v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17, 255),
        6 => (v(0, 2)?, v(2, 2)?, v(4, 2)?, 255),
        8 => (v(0, 2)?, v(2, 2)?, v(4, 2)?, v(6, 2)?),
        _ => return None,
    };
    Some((vec3(srgb8_to_linear(r), srgb8_to_linear(g), srgb8_to_linear(b)), a as f32 / 255.0))
}

/// CSS-style named colours (sRGB 8-bit).
pub const NAMED: &[(&str, [u8; 3])] = &[
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
    ("red", [255, 0, 0]),
    ("green", [0, 128, 0]),
    ("lime", [0, 255, 0]),
    ("blue", [0, 0, 255]),
    ("yellow", [255, 255, 0]),
    ("cyan", [0, 255, 255]),
    ("magenta", [255, 0, 255]),
    ("gray", [128, 128, 128]),
    ("grey", [128, 128, 128]),
    ("silver", [192, 192, 192]),
    ("maroon", [128, 0, 0]),
    ("olive", [128, 128, 0]),
    ("navy", [0, 0, 128]),
    ("purple", [128, 0, 128]),
    ("teal", [0, 128, 128]),
    ("orange", [255, 165, 0]),
    ("pink", [255, 192, 203]),
    ("brown", [139, 69, 19]),
    ("gold", [255, 215, 0]),
    ("beige", [245, 245, 220]),
    ("ivory", [255, 255, 240]),
    ("tan", [210, 180, 140]),
    ("coral", [255, 127, 80]),
    ("salmon", [250, 128, 114]),
    ("khaki", [240, 230, 140]),
    ("indigo", [75, 0, 130]),
    ("violet", [238, 130, 238]),
    ("turquoise", [64, 224, 208]),
    ("skyblue", [135, 206, 235]),
    ("crimson", [220, 20, 60]),
    ("chocolate", [210, 105, 30]),
    ("lavender", [230, 230, 250]),
    ("darkgreen", [0, 100, 0]),
    ("darkblue", [0, 0, 139]),
    ("darkred", [139, 0, 0]),
    ("lightgray", [211, 211, 211]),
    ("darkgray", [64, 64, 64]),
    ("cream", [255, 253, 208]),
];

pub fn named(name: &str) -> Option<Vec3> {
    let n = name.to_ascii_lowercase();
    NAMED.iter().find(|(k, _)| *k == n).map(|(_, c)| vec3(srgb8_to_linear(c[0]), srgb8_to_linear(c[1]), srgb8_to_linear(c[2])))
}

/// Colour of a black-body radiator at `kelvin` (1667..25000 K) as linear sRGB,
/// normalised so the largest channel is 1. Planckian-locus approximation of
/// Kim et al. (2002) followed by the XYZ -> sRGB matrix.
pub fn kelvin(kelvin: f32) -> Vec3 {
    let t = (kelvin as f64).clamp(1667.0, 25000.0);
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    let (cx, cy, cz) = (x / y, 1.0, (1.0 - x - y) / y);
    let r = 3.2406 * cx - 1.5372 * cy - 0.4986 * cz;
    let g = -0.9689 * cx + 1.8758 * cy + 0.0415 * cz;
    let b = 0.0557 * cx - 0.2040 * cy + 1.0570 * cz;
    let (r, g, b) = (r.max(0.0), g.max(0.0), b.max(0.0));
    let m = r.max(g).max(b).max(1e-9);
    vec3((r / m) as f32, (g / m) as f32, (b / m) as f32)
}

/// Linear sRGB -> OKLab (Ottosson 2020).
pub fn oklab(c: Vec3) -> Vec3 {
    let l = 0.412_221_47 * c.x + 0.536_332_55 * c.y + 0.051_445_99 * c.z;
    let m = 0.211_903_5 * c.x + 0.680_699_5 * c.y + 0.107_396_96 * c.z;
    let s = 0.088_302_46 * c.x + 0.281_718_85 * c.y + 0.629_978_7 * c.z;
    let (l, m, s) = (dmath::cbrt(l.max(0.0)), dmath::cbrt(m.max(0.0)), dmath::cbrt(s.max(0.0)));
    vec3(
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    )
}

/// Plain-language description of a linear-sRGB colour, e.g. "dark muted orange (brown)".
/// Deliberately coarse: it is feedback for a model that cannot see the pixel.
pub fn describe(linear: Vec3) -> String {
    let lab = oklab(linear.clamp(0.0, 1.0));
    let (l, a, b) = (lab.x, lab.y, lab.z);
    let chroma = (a * a + b * b).sqrt();
    let light = if l < 0.12 {
        "black"
    } else if l < 0.32 {
        "very dark"
    } else if l < 0.52 {
        "dark"
    } else if l < 0.72 {
        "medium"
    } else if l < 0.9 {
        "light"
    } else {
        "very light"
    };
    if chroma < 0.025 {
        return match light {
            "black" => "black".into(),
            "very light" if l > 0.97 => "white".into(),
            other => format!("{other} gray"),
        };
    }
    if l < 0.12 {
        return "near-black".into();
    }
    let hue = {
        let h = dmath::atan2(b, a).to_degrees();
        if h < 0.0 {
            h + 360.0
        } else {
            h
        }
    };
    let name = if !(20.0..345.0).contains(&hue) {
        "pink-red"
    } else if hue < 45.0 {
        "red"
    } else if hue < 75.0 {
        "orange"
    } else if hue < 115.0 {
        "yellow"
    } else if hue < 160.0 {
        "green"
    } else if hue < 200.0 {
        "teal"
    } else if hue < 235.0 {
        "cyan-blue"
    } else if hue < 280.0 {
        "blue"
    } else if hue < 315.0 {
        "purple"
    } else {
        "magenta"
    };
    let sat = if chroma < 0.07 {
        "muted "
    } else if chroma > 0.17 {
        "vivid "
    } else {
        ""
    };
    let extra = if (name == "orange" || name == "red") && l < 0.6 && chroma < 0.14 {
        " (brown)"
    } else if name == "orange" && l > 0.8 && chroma < 0.09 {
        " (beige/skin)"
    } else if name == "yellow" && l < 0.6 {
        " (olive)"
    } else if name == "pink-red" && l > 0.72 {
        " (pink)"
    } else {
        ""
    };
    format!("{light} {sat}{name}{extra}")
}

/// RGB (any range) -> (hue degrees, saturation, value).
pub fn rgb_to_hsv(c: Vec3) -> (f32, f32, f32) {
    let mx = c.max_comp();
    let mn = c.min_comp();
    let d = mx - mn;
    let h = if d <= 0.0 {
        0.0
    } else if mx == c.x {
        60.0 * (((c.y - c.z) / d) % 6.0)
    } else if mx == c.y {
        60.0 * ((c.z - c.x) / d + 2.0)
    } else {
        60.0 * ((c.x - c.y) / d + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    (h, if mx <= 0.0 { 0.0 } else { d / mx }, mx)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Vec3 {
    let h = ((h % 360.0) + 360.0) % 360.0 / 60.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    vec3(r + m, g + m, b + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb8_roundtrip_is_exact() {
        for v in 0..=255u8 {
            assert_eq!(to_srgb8(srgb8_to_linear(v)), v, "code value {v}");
        }
    }

    #[test]
    fn srgb16_roundtrip_is_exact() {
        for v in (0..=65535u32).step_by(7) {
            let lin = srgb_to_linear(v as f32 / 65535.0);
            assert_eq!(to_srgb16(lin) as u32, v);
        }
    }

    #[test]
    fn hex_and_names() {
        let (c, a) = parse_hex("#ff8000").unwrap();
        assert!((c.x - 1.0).abs() < 1e-6 && c.z == 0.0 && a == 1.0);
        assert_eq!(hex_string(c), "#ff8000");
        assert_eq!(parse_hex("#fff").unwrap().0, Vec3::ONE);
        assert!((parse_hex("#00000080").unwrap().1 - 128.0 / 255.0).abs() < 1e-6);
        assert!(parse_hex("#ggg").is_none() && parse_hex("#12345").is_none());
        assert_eq!(hex_string(named("orange").unwrap()), "#ffa500");
    }

    #[test]
    fn kelvin_trend() {
        let warm = kelvin(2700.0);
        let neutral = kelvin(6500.0);
        let cool = kelvin(12000.0);
        assert!(warm.x > warm.y && warm.y > warm.z, "{warm:?}");
        assert!((neutral.x - neutral.z).abs() < 0.08 && neutral.min_comp() > 0.9, "{neutral:?}");
        assert!(cool.z > cool.x, "{cool:?}");
    }

    #[test]
    fn descriptions() {
        let d = |h: &str| describe(parse_hex(h).unwrap().0);
        assert_eq!(d("#000000"), "black");
        assert_eq!(d("#ffffff"), "white");
        assert!(d("#808080").contains("gray"));
        assert!(d("#ff0000").contains("red"), "{}", d("#ff0000"));
        assert!(d("#00a000").contains("green"));
        assert!(d("#1030e0").contains("blue"), "{}", d("#1030e0"));
        assert!(d("#ffe000").contains("yellow"));
        assert!(d("#8b4513").contains("brown"), "{}", d("#8b4513"));
        assert!(d("#ff8c00").contains("orange"), "{}", d("#ff8c00"));
    }

    #[test]
    fn hsv_roundtrip() {
        for c in [vec3(0.2, 0.5, 0.9), vec3(0.9, 0.1, 0.1), vec3(0.3, 0.3, 0.3), vec3(0.1, 0.8, 0.4)] {
            let (h, s, v) = rgb_to_hsv(c);
            assert!((hsv_to_rgb(h, s, v) - c).length() < 1e-5);
        }
    }
}
