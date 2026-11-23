//! cargo run --release --example fwprof: draw times of the fireworks show's layer kinds at 1080p
//! (bounded drawing, as with an EffectCraft that takes frame_bounds), median of 30 frames.
use particraft::{DEFS, Kind, Params, Scratch, Sprites, State, render, step};
use std::time::Instant;

type Sets<'a> = &'a [(&'a str, &'a [f64])];

fn flat(sets: Sets) -> Vec<f64> {
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
            v[at..at + x.len()].copy_from_slice(x);
        }
    }
    v
}

fn main() {
    let (w, h) = (1920, 1080);
    let pos: &[f64] = &[960.0, 380.0, 0.0];
    // (name, seconds simulated first, settings)
    let scenes: &[(&str, f64, Sets)] = &[
        (
            "one dot",
            1.0,
            &[
                ("rate", &[1.0]),
                ("emitter_behavior", &[1.0]),
                ("velocity", &[0.0]),
                ("position", pos),
            ],
        ),
        (
            "smoke (16 puffs 120)",
            3.0,
            &[
                ("emitter_behavior", &[1.0]),
                ("rate", &[16.0]),
                ("position", pos),
                ("velocity", &[110.0]),
                ("velocity_random", &[60.0]),
                ("air_resistance", &[2.0]),
                ("gravity", &[-4.0]),
                ("life", &[8.0]),
                ("life_random", &[30.0]),
                ("particle_type", &[2.0]),
                ("size", &[120.0]),
                ("size_random", &[50.0]),
                ("opacity", &[3.5]),
                ("opacity_over_life", &[4.0]),
                ("turbulence", &[10.0]),
                ("turb_scale", &[6.0]),
                ("random_rotation", &[100.0]),
            ],
        ),
        (
            "peony (stars, trails, mb)",
            1.0,
            &[
                ("emitter_behavior", &[1.0]),
                ("rate", &[1700.0]),
                ("position", pos),
                ("velocity", &[440.0]),
                ("velocity_random", &[14.0]),
                ("life", &[2.3]),
                ("life_random", &[30.0]),
                ("gravity", &[35.0]),
                ("air_resistance", &[1.5]),
                ("particle_type", &[1.0]),
                ("size", &[3.4]),
                ("size_random", &[35.0]),
                ("glow_size", &[280.0]),
                ("glow_opacity", &[28.0]),
                ("opacity_over_life", &[4.0]),
                ("size_over_life", &[1.0]),
                ("set_color", &[1.0]),
                ("shutter_angle", &[540.0]),
                ("motion_blur", &[1.0]),
                ("blend_mode", &[1.0]),
                ("turbulence", &[8.0]),
                ("turb_scale", &[30.0]),
                ("aux_emit", &[1.0]),
                ("aux_type", &[1.0]),
                ("aux_blend_mode", &[1.0]),
                ("aux_color_from_main", &[100.0]),
                ("aux_rate", &[34.0]),
                ("aux_velocity", &[6.0]),
                ("aux_life", &[0.4]),
                ("aux_life_random", &[50.0]),
                ("aux_size", &[1.8]),
                ("aux_size_random", &[40.0]),
                ("aux_opacity", &[70.0]),
                ("aux_gravity", &[20.0]),
                ("aux_air_resistance", &[2.0]),
            ],
        ),
    ];
    let dt = 1.0f32 / 30.0;
    let mut px = vec![[0.0f32; 4]; w * h];
    let mut sc = Scratch::default();
    let sprites = Sprites::default();
    for (name, at, sets) in scenes {
        let p = Params::from_flat(&flat(sets));
        let mut st = State::default();
        for _ in 0..(*at / dt as f64) as usize {
            step(&mut st, &p, &p, dt);
        }
        let mut draw = vec![];
        let mut b = [0; 4];
        for _ in 0..30 {
            step(&mut st, &p, &p, dt);
            let t = Instant::now();
            b = render(
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
                true,
            );
            draw.push(t.elapsed().as_secs_f64() * 1e3);
        }
        draw.sort_by(f64::total_cmp);
        println!(
            "{name:28} particles {:5} + aux {:5}  draw {:6.2} ms  bounds {b:?}",
            st.ps.len(),
            st.aux.len(),
            draw[15]
        );
    }
}
