//! What `vl inspect` reports, and what it must stay quiet about.
//!
//! A text-only model acts on every finding. A finding about something that is not a
//! mistake costs it a round and can talk it out of a good picture (first experiment:
//! a lamp "outside the picture", table legs "hidden" by their own table top).
use vl_lang::compile;
use vl_render::inspect::inspect;

fn findings(src: &str) -> Vec<(String, String)> {
    let c = compile("test.vl", src);
    let errors: Vec<String> = c.diags.iter().filter(|d| d.severity == vl_core::diag::Severity::Error).map(|d| format!("{} {}", d.code, d.message)).collect();
    assert!(errors.is_empty(), "unexpected errors:\n{}", errors.join("\n"));
    inspect(c.scene.as_ref().unwrap(), 2).findings.iter().map(|f| (f.code.to_string(), f.message.clone())).collect()
}

#[test]
fn real_occlusion_is_reported_and_normal_arrangements_are_not() {
    let f = findings(
        r#"
        render { size: [640, 360] }
        plane ground { }
        box wall { size: [1, 1, 0.1], at: [0, 0.5] }
        sphere ball { radius: 0.1, at: [0, -0.6] }
        sphere peek { radius: 0.3, at: [0.45, -0.6] }
        group table { at: [3, 0]
          box top { size: [1, 0.05, 1], at: [0, 0.725, 0] }
          box leg { size: [0.05, 0.7, 0.05], at: [0.4, -0.4] }
          box leg { size: [0.05, 0.7, 0.05], at: [-0.4, -0.4] }
        }
        camera { from: [0, 0.5, 4], look_at: [1.5, 0.4, 0], fov: 50 }
    "#,
    );
    assert!(f.iter().any(|x| x.0 == "V102" && x.1.contains("'ball'") && x.1.contains("'wall'")), "{f:?}");
    assert!(f.iter().any(|x| x.0 == "V103" && x.1.contains("'peek'")), "{f:?}");
    // the legs behind the table top belong to the table; nothing is said about parts
    assert_eq!(f.len(), 2, "{f:?}");
    assert!(f.iter().all(|x| !x.1.contains("table")), "{f:?}");
}

#[test]
fn piles_contents_and_things_behind_glass_are_not_called_hidden() {
    let f = findings(
        r#"
        render { size: [1280, 720] }
        plane ground { }
        box counter { size: [1.6, 0.9, 0.7] }
        cup bowl { radius: 6cm, top_radius: 15cm, height: 9cm, wall: 6mm, on: counter, at: [-0.1, 0] }
        for i in 0..6 {
            sphere orange { radius: 3.6cm, in: bowl, color: orange }
        }
        cup glass { radius: 3.3cm, height: 12cm, wall: 2mm, on: counter, at: [0.25, 0.1], material: glass }
        for i in 0..3 {
            sphere ice { radius: 1.5cm, in: glass, material: ice }
        }
        box book { size: [0.2, 0.03, 0.14], on: counter, at: [-0.5, 0.1] }
        box book2 { size: [0.2, 0.03, 0.14], on: book, rotate: 10 }
        camera { look_at: [0.05, 0.95, 0.05], azimuth: 20, elevation: 30, distance: 1.1, fov: 42 }
    "#,
    );
    // the counter runs out of the picture: it is what everything stands on, not the subject
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn a_lamp_out_of_view_is_fine_and_an_object_out_of_view_is_not() {
    let f = findings(
        r#"
        render { size: [640, 360] }
        plane ground { }
        sphere ball { radius: 0.2 }
        sphere lost { radius: 0.2, at: [0, 0.2, 30] }
        light lamp { at: [0, 3, 6], power: 3000 }
        camera { look_at: ball, from: [0, 1, 3] }
    "#,
    );
    assert!(f.iter().any(|x| x.0 == "V101" && x.1.contains("'lost'")), "{f:?}");
    assert!(f.iter().all(|x| !x.1.contains("lamp")), "{f:?}");
}
