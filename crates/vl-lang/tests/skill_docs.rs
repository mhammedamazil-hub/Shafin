//! The skill's documentation is checked against the implementation: every ```vl block must
//! compile without errors, and every preset and kind it lists must exist.
use std::path::Path;

fn read(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn vl_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        match &mut cur {
            None if line.trim() == "```vl" => cur = Some(String::new()),
            Some(_) if line.trim() == "```" => out.push(cur.take().unwrap()),
            Some(b) => {
                b.push_str(line);
                b.push('\n');
            }
            None => {}
        }
    }
    out
}

#[test]
fn every_vl_example_in_the_skill_compiles_cleanly() {
    let mut n = 0;
    for file in ["skills/vl-scene/references/language.md", "skills/vl-scene/SKILL.md"] {
        for (i, block) in vl_blocks(&read(file)).iter().enumerate() {
            let c = vl_lang::compile(file, block);
            assert!(c.ok() && c.diags.is_empty(), "{file}, vl block {}: {:?}\n{block}", i + 1, c.diags);
            n += 1;
        }
    }
    assert!(n >= 2, "expected at least two examples, found {n}");
}

#[test]
fn every_statement_form_shown_in_the_reference_is_accepted() {
    // the one-line forms shown in plain code blocks, completed into programs
    let cases = [
        "sphere s { radius: 0.5 }",
        "box b { size: [1, 2, 3], round: 0.01 }",
        "cylinder c { radius: 0.5, height: 1 }",
        "cone c { radius: 0.5, top_radius: 0, height: 1 }",
        "torus t { radius: 0.5, tube: 0.1 }",
        "capsule c { radius: 0.25, height: 1 }",
        "plane p { }",
        "disk d { radius: 0.5 }\nrect r { size: [1, 2] }",
        "box table { }\nsphere s { on: table, at: [0.1, 0.2] }\nsphere q { right_of: table, gap: 0.1 }\nsphere a { at: [1, 2] }\nsphere z { at: [1, 2, 3], rotate: 30, scale: 2 }\nsphere y { rotate: [10, 20, 30] }",
        "box table { }\nlet a = table.top + table.bottom + table.width + table.height + table.depth + table.center.x + table.size.y",
        "sphere s { color: #ff8800 }\nsphere t { color: red }\nsphere u { color: rgb(0.2, 0.4, 0.6), material: wood }",
        "sky { sun_elevation: 40, sun_azimuth: 135 }",
        "environment { color: white, strength: 500 }",
        "box table { }\nlight lamp { at: [0, 2, 0], power: 800, temperature: 2700 }\nlight panel { shape: rect, size: [1, 0.5], at: [0, 2.4, 0], aim: table, power: 3000 }",
        "box table { }\ncamera { look_at: table, azimuth: 30, elevation: 20, fov: 35 }",
        "sphere vase { }\ncamera { look_at: vase, from: [2, 1.5, 3], f_stop: 2.8, focus: vase }",
        "render { size: [1280, 720], samples: 64 }",
        "let x = 1.5\nfor i in 0..5 { sphere s { at: [i, 0] } }\nif x > 1 { sphere a { } } else { sphere b { } }",
        "define chair(height = 0.45) { box seat { size: [0.4, 0.04, 0.4], at: [0, height, 0] } }\nchair c1 { at: [1, 0] }\ngroup g { sphere a { } sphere b { on: a } }",
    ];
    for src in cases {
        let c = vl_lang::compile("reference", src);
        assert_eq!(c.error_count(), 0, "{src}\n{:?}", c.diags);
    }
}

#[test]
fn listed_presets_functions_and_properties_exist() {
    let text = read("skills/vl-scene/references/language.md");
    let section = |start: &str| {
        let i = text.find(start).unwrap_or_else(|| panic!("'{start}' not found in the reference"));
        let rest = &text[i + start.len()..];
        rest[..rest.find('`').unwrap()].to_string()
    };
    for name in section("Presets: `").split_whitespace() {
        assert!(vl_lang::presets::preset(name, None).is_some(), "the reference lists preset '{name}', which does not exist");
    }
    // ... and the reference does not hide presets that exist (apart from 'light', which is set through emission)
    let listed = section("Presets: `");
    for (name, _) in vl_lang::presets::PRESETS {
        assert!(*name == "light" || listed.split_whitespace().any(|n| n == *name), "preset '{name}' is missing from the reference");
    }
    let funcs = vl_lang::value::all_function_names();
    for f in section("Texture functions: `").split_whitespace() {
        let name = f.split('(').next().unwrap();
        if name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && !name.is_empty() && !["p", "octaves", "a", "b", "t", "x", "edge"].contains(&name) {
            assert!(funcs.contains(&name), "the reference lists function '{name}', which does not exist");
        }
    }
    for prop in section("Material properties: `").split_whitespace() {
        assert!(vl_lang::kinds::MATERIAL_PROPS.contains(&prop), "the reference lists material property '{prop}', which does not exist");
    }
    // Agent Skills format: front matter with a name equal to the folder and a bounded description
    let skill = read("skills/vl-scene/SKILL.md");
    assert!(skill.starts_with("---\nname: vl-scene\n"));
    let desc = skill.lines().find(|l| l.starts_with("description: ")).unwrap();
    assert!(desc.len() - "description: ".len() <= 1024);
    assert!(skill.lines().count() < 500);
}
