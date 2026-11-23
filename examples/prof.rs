//! cargo run --release --example prof: times simulation and rendering of heavy scenes at 1080p
//! without EffectCraft or the protocol (median of 30 frames after 3 s of simulation).
use particraft::{DEFS, Kind, Params, Scratch, Sprites, State, render, step};
use std::time::Instant;

fn flat(sets: &[(&str, f64)]) -> Vec<f64> {
    let mut v = vec![];
    for p in DEFS {
        let at = v.len();
        match &p.kind {
            Kind::Slider(x, ..) | Kind::Angle(x) => v.push(*x),
            Kind::Check(b) => v.push(*b as u8 as f64),
            Kind::Popup(_, i) => v.push(*i as f64),
            Kind::Point3(q) => v.extend(q),
            Kind::Color(c) => v.extend(c),
            Kind::Layer => v.push(-1.0),
            Kind::Text(_) | Kind::Gradient(_) => {}
        }
        if let Some((_, x)) = sets.iter().find(|(id, _)| *id == p.id) {
            v[at] = *x;
        }
    }
    v
}

fn main() {
    let scenes: &[(&str, &[(&str, f64)])] = &[
        (
            "30k/s spheres",
            &[("rate", 30000.0), ("size", 2.0), ("velocity", 300.0)],
        ),
        (
            "30k/s + gravity, bounce",
            &[
                ("rate", 30000.0),
                ("size", 2.0),
                ("velocity", 300.0),
                ("gravity", 300.0),
                ("bounce", 1.0),
            ],
        ),
        (
            "30k/s + self-shadows",
            &[
                ("rate", 30000.0),
                ("size", 2.0),
                ("velocity", 300.0),
                ("self_shadow", 1.0),
            ],
        ),
        (
            "30k/s + turbulence",
            &[
                ("rate", 30000.0),
                ("size", 2.0),
                ("velocity", 300.0),
                ("turbulence", 50.0),
            ],
        ),
        (
            "5k/s glow size 6",
            &[
                ("rate", 5000.0),
                ("particle_type", 1.0),
                ("size", 6.0),
                ("velocity", 300.0),
            ],
        ),
        (
            "2k/s puffs size 20",
            &[
                ("rate", 2000.0),
                ("particle_type", 2.0),
                ("size", 20.0),
                ("opacity", 30.0),
                ("velocity", 300.0),
            ],
        ),
        (
            "5k/s stars size 6",
            &[
                ("rate", 5000.0),
                ("particle_type", 4.0),
                ("size", 6.0),
                ("velocity", 300.0),
            ],
        ),
        (
            "5k/s streak particles (7) + mb",
            &[
                ("rate", 5000.0),
                ("particle_type", 5.0),
                ("size", 10.0),
                ("velocity", 300.0),
                ("motion_blur", 1.0),
            ],
        ),
        (
            "same, Add (split glow)",
            &[
                ("rate", 5000.0),
                ("particle_type", 1.0),
                ("size", 6.0),
                ("velocity", 300.0),
                ("blend_mode", 1.0),
            ],
        ),
        (
            "30k/s + motion blur",
            &[
                ("rate", 30000.0),
                ("size", 2.0),
                ("velocity", 600.0),
                ("motion_blur", 1.0),
            ],
        ),
        (
            "20k/s sprites size 6, spinning",
            &[
                ("rate", 20000.0),
                ("velocity", 300.0),
                ("particle_type", 3.0),
                ("size", 6.0),
                ("rotation_speed", 0.5),
                ("random_rotation", 100.0),
            ],
        ),
        (
            "2k/s sprites size 40",
            &[
                ("rate", 2000.0),
                ("velocity", 300.0),
                ("particle_type", 3.0),
                ("size", 40.0),
                ("random_rotation", 100.0),
            ],
        ),
        (
            "2k/s sprites size 20 + motion blur (0.10.0)",
            &[
                ("rate", 2000.0),
                ("velocity", 600.0),
                ("particle_type", 3.0),
                ("size", 20.0),
                ("random_rotation", 100.0),
                ("motion_blur", 1.0),
            ],
        ),
        (
            "2k/s sprites size 20, 30-frame clip, Random - Loop (0.10.0)",
            &[
                ("rate", 2000.0),
                ("velocity", 300.0),
                ("particle_type", 3.0),
                ("size", 20.0),
                ("random_rotation", 100.0),
                ("time_sampling", 6.0),
            ],
        ),
        (
            "2k mains + aux 60/s",
            &[
                ("emitter_behavior", 1.0),
                ("rate", 2000.0),
                ("velocity", 400.0),
                ("life", 4.0),
                ("aux_emit", 1.0),
                ("aux_rate", 60.0),
                ("aux_life", 1.0),
                ("aux_size", 2.0),
                ("aux_velocity", 10.0),
            ],
        ),
        (
            "same + motion blur, Add",
            &[
                ("emitter_behavior", 1.0),
                ("rate", 2000.0),
                ("velocity", 400.0),
                ("life", 4.0),
                ("aux_emit", 1.0),
                ("aux_rate", 60.0),
                ("aux_life", 1.0),
                ("aux_size", 2.0),
                ("aux_velocity", 10.0),
                ("motion_blur", 1.0),
                ("blend_mode", 1.0),
                ("aux_blend_mode", 1.0),
            ],
        ),
    ];
    let (w, h) = (1920usize, 1080usize);
    let mut px = vec![[0.0f32; 4]; w * h];
    let mut sc = Scratch::default();
    let mut sprites = Sprites::default();
    let dt = 1.0 / 30.0;
    // SCENE=<part of a name>: only those scenes
    let only = std::env::var("SCENE").unwrap_or_default();
    for (name, sets) in scenes.iter().filter(|(n, _)| n.contains(only.as_str())) {
        let p = Params::from_flat(&flat(sets));
        let mut st = State::default();
        if sets
            .iter()
            .any(|(id, v)| *id == "particle_type" && *v == 3.0)
        {
            // a 128 px soft round sprite, in a frame-sized image as EC hands it over
            let mut img = vec![[0.0f32; 4]; w * h];
            for y in 0..128 {
                for x in 0..128 {
                    let d = (((x as f32 - 63.5).powi(2) + (y as f32 - 63.5).powi(2)).sqrt() / 64.0)
                        .min(1.0);
                    let a = (1.0 - d * d) * (0.6 + 0.4 * ((x / 8 + y / 8) % 2) as f32);
                    img[(500 + y) * w + 900 + x] = [a, a * 0.8, a * 0.6, a];
                }
            }
            // a 1 s clip of it (the same picture, shifted a pixel a frame) for Time Sampling
            sprites.clear();
            for f in 0..30 {
                let mut sh = vec![[0.0f32; 4]; w * h];
                sh[f..].copy_from_slice(&img[..w * h - f]);
                sprites.insert(f as i64, &sh, w, h);
            }
        }
        for _ in 0..90 {
            step(&mut st, &p, &p, dt);
        }
        let (mut sim, mut draw) = (vec![], vec![]);
        for _ in 0..30 {
            let t = Instant::now();
            step(&mut st, &p, &p, dt);
            sim.push(t.elapsed().as_secs_f64() * 1e3);
            let t = Instant::now();
            render(
                &mut px,
                w,
                h,
                &st,
                &p,
                1.0,
                [0.0; 2],
                [w as f32, h as f32],
                dt,
                &mut sc,
                &sprites,
                None,
                None,
                false,
            );
            draw.push(t.elapsed().as_secs_f64() * 1e3);
        }
        sim.sort_by(f64::total_cmp);
        draw.sort_by(f64::total_cmp);
        println!(
            "{name:24} particles {:6} + aux {:6}  sim {:6.2} ms  draw {:6.2} ms",
            st.ps.len(),
            st.aux.len(),
            sim[15],
            draw[15]
        );
    }
}
