//! A 3D particle system for EffectCraft, its looks and motion fitted
//! to reference renders. The simulation is deterministic: the state at frame k
//! depends only on the parameters at frames 0..=k, so the helper can cache it and resume.
//!
//! Units are layer pixels and seconds. x right, y down, z away from the camera, which is the reference's
//! default comp camera (50 mm: zoom = width · 50/36, at z = -zoom, looking at the layer's centre).

use rayon::prelude::*;
use serde_json::{Value, json};

pub type Px = [f32; 4];

pub enum Kind {
    /// default, min, max, slider max
    Slider(f64, f64, f64, f64),
    Angle(f64),
    Check(bool),
    Popup(&'static [&'static str], usize),
    /// default in layer pixels (EffectCraft has no fractions for point3): The reference's 1080p default
    Point3([f64; 3]),
    Color([f64; 4]),
    /// another layer of the composition (flattens to its id, -1 for none)
    Layer,
    /// a string (a text field; sent apart from the flat values, in `texts`)
    Text(&'static str),
    /// a colour gradient, (position, r, g, b) stops: declared only to hosts listing
    /// "gradient_params" (the newer EffectCraft), sent as JSON in `texts`
    Gradient(&'static [[f64; 4]]),
}
use Kind::*;

pub struct Def {
    pub id: &'static str,
    pub group: &'static str,
    pub name: &'static str,
    pub kind: Kind,
}

const fn d(id: &'static str, group: &'static str, name: &'static str, kind: Kind) -> Def {
    Def {
        id,
        group,
        name,
        kind,
    }
}

const CURVES: &[&str] = &[
    "Constant",
    "Linear Down",
    "Linear Up",
    "Up and Down",
    "Fade In and Out",
];

pub const DEFS: &[Def] = &[
    d(
        "emitter_behavior",
        "Emitter",
        "Emitter Behavior",
        Popup(&["Continuous", "Explode"], 0),
    ),
    d(
        "rate",
        "Emitter",
        "Particles/sec",
        Slider(100.0, 0.0, 1_000_000.0, 1000.0),
    ),
    d(
        "emitter_type",
        "Emitter",
        "Emitter Type",
        Popup(
            &[
                "Point",
                "Box",
                "Sphere",
                "Layer",
                "Light(s)",
                "Text/Mask",
                "3D Model",
            ],
            0,
        ),
    ),
    // Emitter Type 3D Model: an OBJ file, fitted into Emitter Size and turned by X / Y / Z
    // Rotation, its vertices, edges or faces emitting
    d("model_file", "Emitter/3D Model", "OBJ File", Text("")),
    d(
        "model_emit_from",
        "Emitter/3D Model",
        "Emit From",
        Popup(&["Vertices", "Edges", "Faces"], 2),
    ),
    // Emitter Type Text/Mask: the Layer emitter's layer (a text layer, or one with masks), from
    // its outline (Edges) or its filled area (Faces, the Layer emitter itself)
    d(
        "text_emit_from",
        "Emitter/Layer Emitter",
        "Text/Mask Emit From",
        Popup(&["Edges", "Faces"], 0),
    ),
    // Emitter Type Layer: particles are born on the emitter layer's opaque pixels (its depth
    // spread over Emitter Size Z behind it)
    d("emitter_layer", "Emitter/Layer Emitter", "Layer", Layer),
    d(
        "layer_sampling",
        "Emitter/Layer Emitter",
        "Layer Sampling",
        Popup(&["Particle Birth Time", "Still", "Current Time"], 0),
    ),
    // the emitter layer's own transform, which EffectCraft doesn't hand a helper (Position
    // stands in for its position): set these to the layer's
    d(
        "layer_rotation_x",
        "Emitter/Layer Emitter",
        "Layer X Rotation",
        Angle(0.0),
    ),
    d(
        "layer_rotation_y",
        "Emitter/Layer Emitter",
        "Layer Y Rotation",
        Angle(0.0),
    ),
    d(
        "layer_rotation_z",
        "Emitter/Layer Emitter",
        "Layer Z Rotation",
        Angle(0.0),
    ),
    d(
        "layer_scale",
        "Emitter/Layer Emitter",
        "Layer Scale [%]",
        Slider(100.0, 0.0, 10_000.0, 400.0),
    ),
    d(
        "layer_rgb",
        "Emitter/Layer Emitter",
        "Layer RGB Usage",
        Popup(
            &[
                "RGB - Particle Color",
                "None",
                "Lightness - Size",
                "Lightness - Velocity",
                "Lightness - Rotation",
                "RGB - Size Vel Rot",
                "RGB - Size Vel Rot + Col",
                "RGB - XYZ Velocity",
                "RGB - XYZ Velocity + Col",
            ],
            0,
        ),
    ),
    // On a Grid: particles from the cell centres of a Particles in X × Y × Z grid over the Box
    // (or over the layer, the reference's Layer Grid; Z planes over Emitter Size Z), or that grid
    // spread evenly through the Sphere
    d(
        "on_a_grid",
        "Emitter/Distribution",
        "On a Grid",
        Check(false),
    ),
    d(
        "grid_x",
        "Emitter/Distribution",
        "Particles in X",
        Slider(10.0, 1.0, 10_000.0, 100.0),
    ),
    d(
        "grid_y",
        "Emitter/Distribution",
        "Particles in Y",
        Slider(10.0, 1.0, 10_000.0, 100.0),
    ),
    d(
        "grid_z",
        "Emitter/Distribution",
        "Particles in Z",
        Slider(3.0, 1.0, 1000.0, 10.0),
    ),
    d(
        "grid_sequencing",
        "Emitter/Distribution",
        "Sequencing Type",
        Popup(
            &["Periodic Burst", "Traverse", "Random", "Z Sync Traverse"],
            0,
        ),
    ),
    d(
        "position",
        "Emitter",
        "Position",
        Point3([960.0, 540.0, 0.0]),
    ),
    // The reference's Position Subframe: where a moving emitter is between frames, a straight line
    // (Linear) or through 10 samples a frame, joined straight or by a smooth curve. Exact isn't
    // offered (10x Smooth keeps within 0.12 px of a 300 px circle run at 1000 px a frame).
    d(
        "position_subframe",
        "Emitter",
        "Position Subframe",
        Popup(&["Linear", "10x Linear", "10x Smooth"], 0),
    ),
    d(
        "emitter_size_x",
        "Emitter",
        "Emitter Size X",
        Slider(500.0, 0.0, 100_000.0, 2000.0),
    ),
    d(
        "emitter_size_y",
        "Emitter",
        "Emitter Size Y",
        Slider(500.0, 0.0, 100_000.0, 2000.0),
    ),
    d(
        "emitter_size_z",
        "Emitter",
        "Emitter Size Z",
        Slider(500.0, 0.0, 100_000.0, 2000.0),
    ),
    d(
        "direction",
        "Emitter",
        "Direction",
        Popup(
            &[
                "Uniform",
                "Directional",
                "Bi-Directional",
                "Disc",
                "Outwards",
            ],
            0,
        ),
    ),
    d(
        "direction_spread",
        "Emitter",
        "Direction Spread [%]",
        Slider(20.0, 0.0, 100.0, 100.0),
    ),
    d("x_rotation", "Emitter", "X Rotation", Angle(0.0)),
    d("y_rotation", "Emitter", "Y Rotation", Angle(0.0)),
    d("z_rotation", "Emitter", "Z Rotation", Angle(0.0)),
    d(
        "velocity",
        "Emitter",
        "Velocity",
        Slider(100.0, -100_000.0, 100_000.0, 1000.0),
    ),
    d(
        "velocity_random",
        "Emitter",
        "Velocity Random [%]",
        Slider(20.0, 0.0, 1000.0, 100.0),
    ),
    d(
        "velocity_from_motion",
        "Emitter",
        "Velocity from Emitter Motion [%]",
        Slider(20.0, -1000.0, 1000.0, 100.0),
    ),
    d(
        "random_seed",
        "Emitter",
        "Random Seed",
        Slider(100_000.0, 0.0, 1_000_000.0, 1_000_000.0),
    ),
    d(
        "life",
        "Particle",
        "Life [sec]",
        Slider(3.0, 0.0, 1000.0, 10.0),
    ),
    d(
        "life_random",
        "Particle",
        "Life Random [%]",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "particle_type",
        "Particle",
        "Particle Type",
        Popup(
            &["Sphere", "Glow Sphere", "Puff", "Sprite", "Star", "Streaks"],
            0,
        ),
    ),
    d("sprite_layer", "Particle/Sprite", "Layer", Layer),
    d(
        "sprite_mode",
        "Particle/Sprite",
        "Sprite Color",
        Popup(&["Normal", "Colorize", "Fill"], 0),
    ),
    // which frame of an animated sprite each particle shows (the reference's Time Sampling; its
    // default is Random - Loop, but EffectCraft can't tell the clip's length, so Clip Length
    // must be set for every mode but Current Time and Birth Time)
    d(
        "time_sampling",
        "Particle/Sprite",
        "Time Sampling",
        Popup(
            &[
                "Current Time",
                "Start at Birth - Play Once",
                "Start at Birth - Loop",
                "Start at Birth - Stretch",
                "Random - Still Frame",
                "Random - Play Once",
                "Random - Loop",
                "Split Clip - Play Once",
                "Split Clip - Loop",
                "Split Clip - Stretch",
                "Birth Time - Still Frame",
            ],
            0,
        ),
    ),
    d(
        "clip_length",
        "Particle/Sprite",
        "Clip Length [sec]",
        Slider(1.0, 0.0, 3600.0, 10.0),
    ),
    d(
        "clip_count",
        "Particle/Sprite",
        "Number of Clips",
        Slider(2.0, 1.0, 1000.0, 20.0),
    ),
    d(
        "sprite_seed",
        "Particle/Sprite",
        "Random Seed",
        Slider(1.0, 0.0, 200_000.0, 1000.0),
    ),
    d("rotate_z", "Particle/Rotation", "Rotate Z", Angle(0.0)),
    d(
        "random_rotation",
        "Particle/Rotation",
        "Random Rotation",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "rotation_speed",
        "Particle/Rotation",
        "Rotation Speed [rev/sec]",
        Slider(0.0, -100.0, 100.0, 5.0),
    ),
    d(
        "random_speed_rotate",
        "Particle/Rotation",
        "Random Speed Rotate",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "sphere_feather",
        "Particle",
        "Sphere Feather",
        Slider(50.0, 0.0, 100.0, 100.0),
    ),
    d(
        "size",
        "Particle",
        "Size",
        Slider(5.0, 0.0, 10_000.0, 100.0),
    ),
    d(
        "size_random",
        "Particle",
        "Size Random [%]",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "size_over_life",
        "Particle",
        "Size over Life",
        Popup(CURVES, 0),
    ),
    d(
        "opacity",
        "Particle",
        "Opacity",
        Slider(100.0, 0.0, 100.0, 100.0),
    ),
    d(
        "opacity_random",
        "Particle",
        "Opacity Random [%]",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "opacity_over_life",
        "Particle",
        "Opacity over Life",
        Popup(CURVES, 0),
    ),
    d(
        "set_color",
        "Particle",
        "Set Color",
        Popup(&["At Start", "Over Life", "Random from Gradient"], 0),
    ),
    d("color", "Particle", "Color", Color([1.0, 1.0, 1.0, 1.0])),
    d(
        "color_random",
        "Particle",
        "Color Random",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "end_color",
        "Particle",
        "Gradient End Color",
        Color([1.0, 0.3, 0.0, 1.0]),
    ),
    // Set Color Over Life / Random from Gradient: The reference's Color over Life gradient, its
    // opacity stops fading the particles too (hosts without gradients use Color to Gradient End
    // Color)
    d(
        "color_over_life",
        "Particle",
        "Color over Life",
        Gradient(&[[0.0, 1.0, 1.0, 1.0], [1.0, 1.0, 0.3, 0.0]]),
    ),
    d(
        "blend_mode",
        "Particle",
        "Blend Mode",
        Popup(&["Normal", "Add", "Screen", "Lighten"], 0),
    ),
    d(
        "glow_size",
        "Particle/Glow",
        "Glow Size",
        Slider(300.0, 0.0, 10_000.0, 1000.0),
    ),
    d(
        "glow_opacity",
        "Particle/Glow",
        "Glow Opacity",
        Slider(25.0, 0.0, 100.0, 100.0),
    ),
    d(
        "glow_feather",
        "Particle/Glow",
        "Glow Feather",
        Slider(100.0, 0.0, 100.0, 100.0),
    ),
    d(
        "glow_blend_mode",
        "Particle/Glow",
        "Glow Blend Mode",
        Popup(&["Normal", "Add", "Screen"], 0),
    ),
    d(
        "streak_count",
        "Particle/Streaks",
        "Number of Streaks",
        Slider(7.0, 1.0, 1000.0, 50.0),
    ),
    d(
        "streak_size",
        "Particle/Streaks",
        "Streak Size",
        Slider(60.0, 0.0, 1000.0, 200.0),
    ),
    d(
        "streak_seed",
        "Particle/Streaks",
        "Streaks Random Seed",
        Slider(0.0, 0.0, 100_000.0, 1000.0),
    ),
    d(
        "gravity",
        "Physics",
        "Gravity",
        Slider(0.0, -100_000.0, 100_000.0, 1000.0),
    ),
    d(
        "air_resistance",
        "Physics",
        "Air Resistance",
        Slider(0.5, 0.0, 100.0, 10.0),
    ),
    d(
        "wind_x",
        "Physics",
        "Wind X",
        Slider(0.0, -100_000.0, 100_000.0, 1000.0),
    ),
    d(
        "wind_y",
        "Physics",
        "Wind Y",
        Slider(0.0, -100_000.0, 100_000.0, 1000.0),
    ),
    d(
        "wind_z",
        "Physics",
        "Wind Z",
        Slider(0.0, -100_000.0, 100_000.0, 1000.0),
    ),
    d(
        "turbulence",
        "Physics/Air Turbulence",
        "Affect Position",
        Slider(0.0, 0.0, 10_000.0, 1000.0),
    ),
    d(
        "turb_scale",
        "Physics/Air Turbulence",
        "Scale",
        Slider(10.0, 0.1, 1000.0, 100.0),
    ),
    d(
        "turb_complexity",
        "Physics/Air Turbulence",
        "Complexity",
        Slider(3.0, 1.0, 8.0, 8.0),
    ),
    d(
        "turb_octave_mult",
        "Physics/Air Turbulence",
        "Octave Multiplier",
        Slider(0.5, 0.0, 1.0, 1.0),
    ),
    d(
        "turb_octave_scale",
        "Physics/Air Turbulence",
        "Octave Scale",
        Slider(1.5, 1.0, 4.0, 4.0),
    ),
    d(
        "turb_evolution",
        "Physics/Air Turbulence",
        "Evolution Speed",
        Slider(50.0, 0.0, 1000.0, 200.0),
    ),
    d(
        "turb_offset",
        "Physics/Air Turbulence",
        "Evolution Offset",
        Slider(0.0, -100_000.0, 100_000.0, 1000.0),
    ),
    d(
        "turb_offset_x",
        "Physics/Air Turbulence",
        "Offset X",
        Slider(0.0, -100_000.0, 100_000.0, 2000.0),
    ),
    d(
        "turb_offset_y",
        "Physics/Air Turbulence",
        "Offset Y",
        Slider(0.0, -100_000.0, 100_000.0, 2000.0),
    ),
    d(
        "turb_offset_z",
        "Physics/Air Turbulence",
        "Offset Z",
        Slider(0.0, -100_000.0, 100_000.0, 2000.0),
    ),
    // The reference bounces off a comp layer: Floor Layer, where EffectCraft says where it is (the
    // newer EffectCraft's layer_transform). Otherwise the floor is placed by the controls below,
    // with comp layer conventions (a layer facing the camera turned X Rotation 90 is a horizontal
    // floor).
    d("bounce", "Physics/Bounce", "Enable Bounce", Check(false)),
    d("floor_layer", "Physics/Bounce", "Floor Layer", Layer),
    d(
        "floor_position",
        "Physics/Bounce",
        "Floor Position",
        Point3([960.0, 900.0, 0.0]),
    ),
    d(
        "floor_x_rotation",
        "Physics/Bounce",
        "Floor X Rotation",
        Angle(90.0),
    ),
    d(
        "floor_y_rotation",
        "Physics/Bounce",
        "Floor Y Rotation",
        Angle(0.0),
    ),
    d(
        "floor_z_rotation",
        "Physics/Bounce",
        "Floor Z Rotation",
        Angle(0.0),
    ),
    d(
        "floor_mode",
        "Physics/Bounce",
        "Bounce Mode",
        Popup(&["Surface", "Layer Size", "Layer Alpha"], 0),
    ),
    d(
        "floor_width",
        "Physics/Bounce",
        "Floor Width",
        Slider(1920.0, 0.0, 100_000.0, 4000.0),
    ),
    d(
        "floor_height",
        "Physics/Bounce",
        "Floor Height",
        Slider(1080.0, 0.0, 100_000.0, 4000.0),
    ),
    d(
        "collision_event",
        "Physics/Bounce",
        "Collision Event",
        Popup(&["Bounce", "Slide", "Stick", "Kill"], 0),
    ),
    d(
        "bounce_strength",
        "Physics/Bounce",
        "Bounce",
        Slider(50.0, 0.0, 1000.0, 100.0),
    ),
    d(
        "bounce_random",
        "Physics/Bounce",
        "Bounce Random",
        Slider(30.0, 0.0, 100.0, 100.0),
    ),
    d(
        "slide_friction",
        "Physics/Bounce",
        "Slide Friction",
        Slider(80.0, 0.0, 100.0, 100.0),
    ),
    // Bounce 2: another layer (a wall), with its own settings; needs an EffectCraft with
    // layer_transform (no stand-in controls)
    d("bounce2_layer", "Physics/Bounce 2", "Layer", Layer),
    d(
        "bounce2_mode",
        "Physics/Bounce 2",
        "Bounce Mode",
        Popup(&["Surface", "Layer Size", "Layer Alpha"], 0),
    ),
    d(
        "bounce2_event",
        "Physics/Bounce 2",
        "Collision Event",
        Popup(&["Bounce", "Slide", "Stick", "Kill"], 0),
    ),
    d(
        "bounce2_strength",
        "Physics/Bounce 2",
        "Bounce",
        Slider(50.0, 0.0, 1000.0, 100.0),
    ),
    d(
        "bounce2_random",
        "Physics/Bounce 2",
        "Bounce Random",
        Slider(30.0, 0.0, 100.0, 100.0),
    ),
    d(
        "bounce2_friction",
        "Physics/Bounce 2",
        "Slide Friction",
        Slider(80.0, 0.0, 100.0, 100.0),
    ),
    // Bounce 3: another layer (a wall), with its own settings; needs an EffectCraft with
    // layer_transform (no stand-in controls)
    d("bounce3_layer", "Physics/Bounce 3", "Layer", Layer),
    d(
        "bounce3_mode",
        "Physics/Bounce 3",
        "Bounce Mode",
        Popup(&["Surface", "Layer Size", "Layer Alpha"], 0),
    ),
    d(
        "bounce3_event",
        "Physics/Bounce 3",
        "Collision Event",
        Popup(&["Bounce", "Slide", "Stick", "Kill"], 0),
    ),
    d(
        "bounce3_strength",
        "Physics/Bounce 3",
        "Bounce",
        Slider(50.0, 0.0, 1000.0, 100.0),
    ),
    d(
        "bounce3_random",
        "Physics/Bounce 3",
        "Bounce Random",
        Slider(30.0, 0.0, 100.0, 100.0),
    ),
    d(
        "bounce3_friction",
        "Physics/Bounce 3",
        "Slide Friction",
        Slider(80.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_emit",
        "Trails",
        "Emit",
        Popup(&["Off", "Continuously"], 0),
    ),
    d(
        "aux_start",
        "Trails",
        "Start Emit [% of Life]",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_stop",
        "Trails",
        "Stop Emit [% of Life]",
        Slider(100.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_rate",
        "Trails",
        "Particles/sec",
        Slider(10.0, 0.0, 100_000.0, 1000.0),
    ),
    d(
        "aux_velocity",
        "Trails",
        "Particle Velocity",
        Slider(0.0, -100_000.0, 100_000.0, 1000.0),
    ),
    d(
        "aux_inherit",
        "Trails",
        "Inherit Main Velocity [%]",
        Slider(0.0, -1000.0, 1000.0, 100.0),
    ),
    d(
        "aux_life",
        "Trails",
        "Life [sec]",
        Slider(0.5, 0.0, 1000.0, 10.0),
    ),
    d(
        "aux_life_random",
        "Trails",
        "Life Random [%]",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_type",
        "Trails",
        "Type",
        Popup(&["Sphere", "Glow Sphere", "Puff"], 0),
    ),
    d(
        "aux_feather",
        "Trails",
        "Feather",
        Slider(30.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_size",
        "Trails",
        "Size",
        Slider(5.0, 0.0, 10_000.0, 100.0),
    ),
    d(
        "aux_size_random",
        "Trails",
        "Size Random [%]",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_size_over_life",
        "Trails",
        "Size over Life",
        Popup(CURVES, 1),
    ),
    d(
        "aux_opacity",
        "Trails",
        "Opacity",
        Slider(50.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_opacity_random",
        "Trails",
        "Opacity Random [%]",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_opacity_over_life",
        "Trails",
        "Opacity over Life",
        Popup(CURVES, 1),
    ),
    d(
        "aux_set_color",
        "Trails",
        "Set Color",
        Popup(&["At Start", "Over Life", "Random from Gradient"], 0),
    ),
    d("aux_color", "Trails", "Color", Color([1.0, 1.0, 1.0, 1.0])),
    d(
        "aux_end_color",
        "Trails",
        "Gradient End Color",
        Color([1.0, 0.3, 0.0, 1.0]),
    ),
    d(
        "aux_color_over_life",
        "Trails",
        "Color over Life",
        Gradient(&[[0.0, 1.0, 1.0, 1.0], [1.0, 1.0, 0.3, 0.0]]),
    ),
    d(
        "aux_color_from_main",
        "Trails",
        "Color From Main [%]",
        Slider(0.0, 0.0, 100.0, 100.0),
    ),
    d(
        "aux_blend_mode",
        "Trails",
        "Blend Mode",
        Popup(&["Normal", "Add", "Screen", "Lighten"], 0),
    ),
    d(
        "aux_gravity",
        "Trails",
        "Gravity",
        Slider(0.0, -100_000.0, 100_000.0, 1000.0),
    ),
    d(
        "aux_air_resistance",
        "Trails",
        "Air Resistance",
        Slider(0.0, 0.0, 100.0, 10.0),
    ),
    d(
        "aux_turbulence",
        "Trails",
        "Turbulence Position",
        Slider(0.0, 0.0, 10_000.0, 1000.0),
    ),
    // The reference's Lighting: the comp's lights (newer EffectCraft's comp_lights) tint every
    // particle by where it is
    d("lighting", "Lighting", "Enable Lighting", Check(false)),
    d(
        "light_falloff",
        "Lighting",
        "Light Falloff",
        Popup(&["None", "Natural"], 1),
    ),
    d(
        "nominal_distance",
        "Lighting",
        "Nominal Distance",
        Slider(250.0, 0.0, 100_000.0, 2000.0),
    ),
    d(
        "ambient",
        "Lighting",
        "Ambient",
        Slider(20.0, 0.0, 1000.0, 100.0),
    ),
    d(
        "diffuse",
        "Lighting",
        "Diffuse",
        Slider(80.0, 0.0, 1000.0, 100.0),
    ),
    // Self-Shadow: each particle's soft shadow darkens the particles behind it
    d(
        "self_shadow",
        "Self-Shadow",
        "Enable Self-Shadow",
        Check(false),
    ),
    d(
        "shadow_softness",
        "Self-Shadow",
        "Softness",
        Slider(60.0, 0.0, 100.0, 100.0),
    ),
    d(
        "shadow_color",
        "Self-Shadow",
        "Shadow Color",
        Color([0.0, 0.0, 0.0, 1.0]),
    ),
    d(
        "shadow_color_strength",
        "Self-Shadow",
        "Color Strength",
        Slider(100.0, 0.0, 100.0, 100.0),
    ),
    d(
        "shadow_opacity",
        "Self-Shadow",
        "Shadow Opacity",
        Slider(5.0, 0.0, 100.0, 100.0),
    ),
    d(
        "shadow_size",
        "Self-Shadow",
        "Adjust Size",
        Slider(100.0, 0.0, 1000.0, 400.0),
    ),
    d(
        "shadow_distance",
        "Self-Shadow",
        "Adjust Distance",
        Slider(100.0, 0.0, 1000.0, 400.0),
    ),
    // The reference's Depth of Field: the comp camera layer's (newer EffectCraft's comp_camera)
    d(
        "depth_of_field",
        "Rendering",
        "Depth of Field",
        Popup(&["Off", "Camera Settings"], 1),
    ),
    d(
        "master_opacity",
        "Rendering",
        "Opacity",
        Slider(100.0, 0.0, 100.0, 100.0),
    ),
    d(
        "motion_blur",
        "Rendering/Motion Blur",
        "Motion Blur",
        Popup(&["Off", "On"], 0),
    ),
    d(
        "shutter_angle",
        "Rendering/Motion Blur",
        "Shutter Angle",
        Slider(360.0, 0.0, 720.0, 720.0),
    ),
    d(
        "shutter_phase",
        "Rendering/Motion Blur",
        "Shutter Phase",
        Slider(0.0, -360.0, 360.0, 360.0),
    ),
    d(
        "mb_samples",
        "Rendering/Motion Blur",
        "Samples",
        Slider(16.0, 1.0, 64.0, 64.0),
    ),
];

pub const ID: &str = "io.github.spoofah.particraft";

/// The effect's manifest; `gradients`: the host takes gradient parameters ("gradient_params").
pub fn manifest(gradients: bool) -> Value {
    let params: Vec<Value> = DEFS
        .iter()
        .filter(|p| gradients || !matches!(p.kind, Gradient(_)))
        .map(|p| {
            let mut v = match &p.kind {
                Slider(def, min, max, smax) => json!({"type": "slider", "default": def, "min": min, "max": max, "sliderMin": min.max(0.0).min(*def), "sliderMax": smax, "decimals": 1}),
                Angle(def) => json!({"type": "angle", "default": def}),
                Check(def) => json!({"type": "checkbox", "default": def}),
                Popup(opts, def) => json!({"type": "popup", "options": opts, "default": def}),
                Point3(def) => json!({"type": "point3", "default": def}),
                Text(def) => json!({"type": "text", "default": def}),
                Color(def) => json!({"type": "color", "default": def}),
                Layer => json!({"type": "layer"}),
                Gradient(stops) => json!({"type": "gradient", "default": {
                    "colors": stops.iter().map(|s| json!([s[0], [s[1], s[2], s[3], 1.0]])).collect::<Vec<_>>(),
                    "opacities": [[0.0, 1.0], [1.0, 1.0]],
                }}),
            };
            v["id"] = json!(p.id);
            // With Color over Life the End Colors only stand in for it on hosts without gradients
            // (renamed, not removed: the flat layout stays the stock one)
            v["name"] = json!(match p.id {
                "end_color" | "aux_end_color" if gradients => "End Color (stock EC only)",
                _ => p.name,
            });
            v["group"] = json!(p.group);
            v
        })
        .collect();
    json!({
        "api": 2, "id": ID, "name": "PartiCraft", "category": "Simulation",
        "version": env!("CARGO_PKG_VERSION"), "author": "Spoofah",
        "description": "A 3D particle system: point, box, sphere, layer, light, text and model emitters; gravity, air resistance, wind, turbulence and bounce; sphere, glow, puff, sprite, star and streak particles; trails, lighting, self-shadow, depth of field and motion blur.",
        "params": params,
    })
}

fn slots(k: &Kind) -> usize {
    match k {
        Point3(_) => 3,
        Color(_) => 4,
        Text(_) | Gradient(_) => 0,
        _ => 1,
    }
}

/// Number of `f64`s EffectCraft sends.
pub fn nflat() -> usize {
    DEFS.iter().map(|p| slots(&p.kind)).sum()
}

/// Flat indices of x and y of every `point3` (they arrive in buffer pixels).
pub fn point_slots() -> Vec<usize> {
    let mut at = 0;
    let mut out = vec![];
    for p in DEFS {
        if matches!(p.kind, Point3(_)) {
            out.extend([at, at + 1]);
        }
        at += slots(&p.kind);
    }
    out
}

/// Parameters in simulation units (layer pixels, seconds, radians, 0–1).
#[derive(Clone, Debug, Default)]
pub struct Params {
    pub explode: bool,
    pub rate: f32,
    pub emitter_type: usize,
    pub pos: [f32; 3],
    /// Position Subframe (0 Linear, 1 10x Linear, 2 10x Smooth) and, filled by the host for a
    /// moving emitter, Position at the step's start + i/10 of it, i = 1..9 (empty: a straight line)
    pub subframe: usize,
    pub sub_pos: Vec<[f32; 3]>,
    pub esize: [f32; 3],
    pub direction: usize,
    pub spread: f32,
    pub rot: [f32; 3],
    pub velocity: f32,
    pub velocity_random: f32,
    pub velocity_from_motion: f32,
    pub seed: u64,
    pub life: f32,
    pub life_random: f32,
    pub ptype: usize,
    pub feather: f32,
    pub size: f32,
    pub size_random: f32,
    pub size_over_life: usize,
    pub opacity: f32,
    pub opacity_random: f32,
    pub opacity_over_life: usize,
    pub set_color: usize,
    pub color: [f32; 3],
    pub color_random: f32,
    pub end_color: [f32; 3],
    pub blend: usize,
    pub glow_size: f32,
    pub glow_opacity: f32,
    pub glow_feather: f32,
    pub glow_blend: usize,
    /// Streaks: dots per particle, their size (fraction) and the pattern's seed
    pub streak_count: usize,
    pub streak_size: f32,
    pub streak_seed: u64,
    pub gravity: f32,
    pub air: f32,
    pub wind: [f32; 3],
    pub turbulence: f32,
    /// Air Turbulence Offset X / Y / Z (px)
    pub turb_shift: [f32; 3],
    pub turb_scale: f32,
    pub turb_complexity: usize,
    pub turb_octave_mult: f32,
    pub turb_octave_scale: f32,
    pub turb_evolution: f32,
    pub turb_offset: f32,
    /// Bounce (empty: off): the floor (Floor Layer or the controls), then Bounce 2 and 3 when
    /// they have a layer (placed by the helper from EffectCraft's layer_transform)
    pub floors: Vec<Floor>,
    pub master_opacity: f32,
    /// Emitter Type Light(s): the comp's lights named "Emitter..." (newer EffectCraft's
    /// comp_lights), placed by the helper for each step
    pub lights: Vec<EmitLight>,
    /// Lighting (off: None): Ambient and Diffuse (1 = 100%), and Nominal Distance for Natural
    /// falloff (None: no falloff)
    pub lighting: Option<Lighting>,
    /// the comp's lights for Lighting at the frame drawn (placed by the helper)
    pub shade: Vec<ShadeLight>,
    /// Self-Shadow (off: None)
    pub self_shadow: Option<SelfShadow>,
    /// the light self-shadows fall away from: the comp light named "Shadow..." at the frame drawn
    /// (placed by the helper); None: one at the camera
    pub shadow_light: Option<ShadeLight>,
    /// Depth of Field: Camera Settings (the camera layer's, when it has it on)
    pub dof: bool,
    /// shutter start and length in frames, and sub-frame samples (0: no motion blur)
    pub shutter: [f32; 2],
    pub mb_samples: usize,
    /// the sprite layer's id (-1: none) and how its colour combines with the particle's
    pub sprite_layer: i64,
    /// Layer emitter: the layer's id (-1: none), sampled once (Still) or at each birth, and
    /// whether its pixels colour the particles
    pub emit_layer: i64,
    pub layer_still: bool,
    /// Text/Mask, Emit From Edges: only the layer's outline emits (emitter_type is then 3)
    pub edges: bool,
    /// drawing Puffs as the built-in cloud texture (render: ptype is then 3)
    pub cloud: bool,
    /// 3D Model: the mesh (loaded by the helper from OBJ File) and Emit From (0 vertices,
    /// 1 edges, 2 faces)
    pub model: Option<std::sync::Arc<Mesh>>,
    pub model_from: usize,
    /// Layer Sampling Current Time: particles take the layer's colour at their birth pixel now
    pub layer_now: bool,
    /// the emitter layer's rotation × scale (None: neither)
    pub layer_xf: Option<[[f32; 3]; 3]>,
    pub layer_color: bool,
    /// Layer RGB Usage: 0 colour, 1 none, 2-4 lightness to size / velocity / rotation, 5-6 RGB to
    /// size, velocity, rotation (6: and colour), 7-8 RGB to XYZ velocity (8: and colour)
    pub layer_usage: usize,
    /// On a Grid (Layer emitter): Particles in X, Y, Z and the Sequencing Type
    pub grid: Option<([u32; 3], usize)>,
    pub sprite_mode: usize,
    /// Time Sampling (0 Current Time … 10 Birth Time), Clip Length (s), Number of Clips, seed
    pub time_sampling: usize,
    pub clip_len: f32,
    pub clip_count: f32,
    pub sprite_seed: u64,
    /// Rotate Z and its random spread (radians), spin (radians per second) and its random share
    pub rot_z: f32,
    pub rot_random: f32,
    pub spin: f32,
    pub spin_random: f32,
    /// Trails: particles each main particle emits along its path
    pub aux_on: bool,
    /// emission window as fractions of the main particle's life
    pub aux_window: [f32; 2],
    pub aux_rate: f32,
    pub aux_velocity: f32,
    pub aux_inherit: f32,
    pub aux_life: f32,
    pub aux_life_random: f32,
    pub aux_type: usize,
    pub aux_feather: f32,
    pub aux_size: f32,
    pub aux_size_random: f32,
    pub aux_size_over_life: usize,
    pub aux_opacity: f32,
    pub aux_opacity_random: f32,
    pub aux_opacity_over_life: usize,
    pub aux_set_color: usize,
    pub aux_color: [f32; 3],
    pub aux_end_color: [f32; 3],
    /// Color over Life gradients, main and aux (from `texts`; None: Color to Gradient End Color)
    pub gradient: Option<std::sync::Arc<Grad>>,
    pub aux_gradient: Option<std::sync::Arc<Grad>>,
    pub aux_from_main: f32,
    pub aux_blend: usize,
    pub aux_gravity: f32,
    pub aux_air: f32,
    pub aux_turbulence: f32,
}

impl Params {
    /// `flat` with point x, y already in layer pixels (see [`to_layer_units`]).
    pub fn from_flat(flat: &[f64]) -> Params {
        let mut at = 0;
        let mut get = std::collections::HashMap::new();
        for p in DEFS {
            get.insert(p.id, at);
            at += slots(&p.kind);
        }
        let f = |id: &str| flat[get[id]] as f32;
        let v3 = |id: &str| {
            let i = get[id];
            [flat[i] as f32, flat[i + 1] as f32, flat[i + 2] as f32]
        };
        let u = |id: &str| flat[get[id]].max(0.0) as usize;
        let pc = |id: &str| f(id) / 100.0;
        Params {
            explode: u("emitter_behavior") == 1,
            rate: f("rate").max(0.0),
            // Text/Mask is the Layer emitter, from its edges or faces
            emitter_type: if u("emitter_type") == 5 {
                3
            } else {
                u("emitter_type")
            },
            edges: u("emitter_type") == 5 && u("text_emit_from") == 0,
            model: None,
            model_from: u("model_emit_from"),
            cloud: false,
            pos: v3("position"),
            subframe: u("position_subframe"),
            sub_pos: Vec::new(),
            esize: [
                f("emitter_size_x"),
                f("emitter_size_y"),
                f("emitter_size_z"),
            ],
            direction: u("direction"),
            spread: pc("direction_spread").clamp(0.0, 1.0),
            rot: [
                f("x_rotation").to_radians(),
                f("y_rotation").to_radians(),
                f("z_rotation").to_radians(),
            ],
            velocity: f("velocity"),
            velocity_random: pc("velocity_random"),
            velocity_from_motion: pc("velocity_from_motion"),
            seed: f("random_seed").max(0.0) as u64,
            life: f("life").max(0.0),
            life_random: pc("life_random").clamp(0.0, 1.0),
            ptype: u("particle_type"),
            sprite_layer: flat[get["sprite_layer"]].round() as i64,
            emit_layer: flat[get["emitter_layer"]].round() as i64,
            layer_still: u("layer_sampling") == 1,
            layer_now: u("layer_sampling") == 2,
            layer_xf: {
                let a = [
                    f("layer_rotation_x").to_radians(),
                    f("layer_rotation_y").to_radians(),
                    f("layer_rotation_z").to_radians(),
                ];
                let k = pc("layer_scale").max(0.0);
                (a != [0.0; 3] || k != 1.0).then(|| layer_matrix(a, k))
            },
            layer_color: matches!(u("layer_rgb"), 0 | 6 | 8),
            layer_usage: u("layer_rgb"),
            grid: (flat[get["on_a_grid"]] > 0.5).then(|| {
                (
                    [
                        f("grid_x").round().clamp(1.0, 1e4) as u32,
                        f("grid_y").round().clamp(1.0, 1e4) as u32,
                        f("grid_z").round().clamp(1.0, 1e3) as u32,
                    ],
                    u("grid_sequencing"),
                )
            }),
            sprite_mode: u("sprite_mode"),
            time_sampling: u("time_sampling"),
            clip_len: f("clip_length").max(0.0),
            clip_count: f("clip_count").round().max(1.0),
            sprite_seed: f("sprite_seed").max(0.0) as u64,
            rot_z: f("rotate_z").to_radians(),
            rot_random: pc("random_rotation").clamp(0.0, 1.0) * std::f32::consts::PI,
            spin: f("rotation_speed") * std::f32::consts::TAU,
            spin_random: pc("random_speed_rotate").clamp(0.0, 1.0),
            feather: pc("sphere_feather").clamp(0.0, 1.0),
            size: f("size").max(0.0),
            size_random: pc("size_random"),
            size_over_life: u("size_over_life"),
            opacity: pc("opacity").clamp(0.0, 1.0),
            opacity_random: pc("opacity_random").clamp(0.0, 1.0),
            opacity_over_life: u("opacity_over_life"),
            set_color: u("set_color"),
            color: v3("color"),
            color_random: pc("color_random"),
            end_color: v3("end_color"),
            blend: u("blend_mode"),
            glow_size: pc("glow_size").max(0.0),
            glow_opacity: pc("glow_opacity").clamp(0.0, 1.0),
            glow_feather: pc("glow_feather").clamp(0.0, 1.0),
            glow_blend: u("glow_blend_mode"),
            streak_count: u("streak_count").clamp(1, 1000),
            streak_size: pc("streak_size").max(0.0),
            streak_seed: f("streak_seed").max(0.0) as u64,
            gravity: f("gravity"),
            air: f("air_resistance").max(0.0),
            wind: [f("wind_x"), f("wind_y"), f("wind_z")],
            turbulence: f("turbulence"),
            turb_shift: [f("turb_offset_x"), f("turb_offset_y"), f("turb_offset_z")],
            turb_scale: f("turb_scale").max(0.1),
            turb_complexity: u("turb_complexity").clamp(1, 8),
            turb_octave_mult: f("turb_octave_mult"),
            turb_octave_scale: f("turb_octave_scale").max(1.0),
            turb_evolution: f("turb_evolution"),
            turb_offset: f("turb_offset"),
            floors: if u("bounce") != 1 {
                Vec::new()
            } else {
                let r = [
                    f("floor_x_rotation").to_radians(),
                    f("floor_y_rotation").to_radians(),
                    f("floor_z_rotation").to_radians(),
                ];
                let (hw, hh) = (0.5 * f("floor_width"), 0.5 * f("floor_height"));
                // turned as a comp layer: Z first, then Y, then X (ref BL4: X 90, Y 30 is a ramp)
                let m = layer_matrix(r, 1.0);
                let col = |j: usize| [m[0][j], m[1][j], m[2][j]];
                let n = col(2).map(|x| -x);
                let floor = Floor {
                    at: v3("floor_position"),
                    at0: v3("floor_position"),
                    n,
                    n0: n,
                    u: col(0),
                    w: col(1),
                    // Layer Alpha without the layer's transform: its size, from the controls
                    rect: (u("floor_mode") >= 1).then_some([-hw, -hh, hw, hh]),
                    alpha_mode: u("floor_mode") == 2,
                    mask: None,
                    event: u("collision_event"),
                    strength: pc("bounce_strength").max(0.0),
                    random: pc("bounce_random").clamp(0.0, 1.0),
                    friction: pc("slide_friction").clamp(0.0, 1.0),
                    layer: flat[get["floor_layer"]].round() as i64,
                    param: "floor_layer",
                    placed: true,
                };
                let mut v = vec![floor];
                for (id, param) in [("bounce2", "bounce2_layer"), ("bounce3", "bounce3_layer")] {
                    let layer = flat[get[param]].round() as i64;
                    if layer < 0 {
                        continue;
                    }
                    let mode = u(&format!("{id}_mode"));
                    v.push(Floor {
                        at: [0.0; 3],
                        at0: [0.0; 3],
                        n: [0.0, 0.0, -1.0],
                        n0: [0.0, 0.0, -1.0],
                        u: [1.0, 0.0, 0.0],
                        w: [0.0, 1.0, 0.0],
                        rect: (mode >= 1).then_some([0.0; 4]),
                        alpha_mode: mode == 2,
                        mask: None,
                        event: u(&format!("{id}_event")),
                        strength: pc(&format!("{id}_strength")).max(0.0),
                        random: pc(&format!("{id}_random")).clamp(0.0, 1.0),
                        friction: pc(&format!("{id}_friction")).clamp(0.0, 1.0),
                        layer,
                        param,
                        placed: false,
                    });
                }
                v
            },
            master_opacity: pc("master_opacity").clamp(0.0, 1.0),
            shutter: [
                f("shutter_phase") / 360.0,
                f("shutter_angle").max(0.0) / 360.0,
            ],
            dof: u("depth_of_field") == 1,
            lights: Vec::new(),
            lighting: (f("lighting") > 0.5).then(|| Lighting {
                ambient: pc("ambient").max(0.0),
                diffuse: pc("diffuse").max(0.0),
                nominal: (u("light_falloff") == 1).then(|| f("nominal_distance").max(0.0)),
            }),
            shade: Vec::new(),
            self_shadow: (f("self_shadow") > 0.5).then(|| SelfShadow {
                softness: pc("shadow_softness").clamp(0.0, 1.0),
                color: v3("shadow_color"),
                strength: pc("shadow_color_strength").clamp(0.0, 1.0),
                opacity: pc("shadow_opacity").clamp(0.0, 1.0),
                size: pc("shadow_size").max(0.0),
                distance: pc("shadow_distance").max(0.0),
            }),
            shadow_light: None,
            mb_samples: if u("motion_blur") == 1 {
                u("mb_samples").clamp(1, 64)
            } else {
                0
            },
            aux_on: u("aux_emit") == 1 && f("aux_rate") > 0.0,
            aux_window: [
                pc("aux_start").clamp(0.0, 1.0),
                pc("aux_stop").clamp(0.0, 1.0),
            ],
            aux_rate: f("aux_rate").max(0.0),
            aux_velocity: f("aux_velocity"),
            aux_inherit: pc("aux_inherit"),
            aux_life: f("aux_life").max(0.0),
            aux_life_random: pc("aux_life_random").clamp(0.0, 1.0),
            aux_type: u("aux_type"),
            aux_feather: pc("aux_feather").clamp(0.0, 1.0),
            aux_size: f("aux_size").max(0.0),
            aux_size_random: pc("aux_size_random"),
            aux_size_over_life: u("aux_size_over_life"),
            aux_opacity: pc("aux_opacity").clamp(0.0, 1.0),
            aux_opacity_random: pc("aux_opacity_random").clamp(0.0, 1.0),
            aux_opacity_over_life: u("aux_opacity_over_life"),
            aux_set_color: u("aux_set_color"),
            aux_color: v3("aux_color"),
            aux_end_color: v3("aux_end_color"),
            gradient: None,
            aux_gradient: None,
            aux_from_main: pc("aux_color_from_main").clamp(0.0, 1.0),
            aux_blend: u("aux_blend_mode"),
            aux_gravity: f("aux_gravity"),
            aux_air: f("aux_air_resistance").max(0.0),
            aux_turbulence: f("aux_turbulence"),
        }
    }
}

impl Params {
    /// These parameters with the aux particles' look in place of the main ones (for drawing them).
    fn aux_look(&self) -> Params {
        Params {
            ptype: self.aux_type,
            feather: self.aux_feather,
            size_over_life: self.aux_size_over_life,
            opacity_over_life: self.aux_opacity_over_life,
            set_color: self.aux_set_color,
            color: self.aux_color,
            end_color: self.aux_end_color,
            gradient: self.aux_gradient.clone(),
            self_shadow: None,
            blend: self.aux_blend,
            ..self.clone()
        }
    }
}

/// EffectCraft sends point x, y in buffer pixels; the simulation keeps layer pixels so a change
/// of preview resolution doesn't restart it.
pub fn to_layer_units(flat: &mut [f64], scale: f64, origin: [f64; 2]) {
    for (n, i) in point_slots().into_iter().enumerate() {
        flat[i] = (flat[i] - origin[n % 2]) / scale;
    }
}

// ---------------------------------------------------------------------------------------------
// Random numbers: a hash of (seed, particle id, channel), so a particle is the same whatever
// order or frame the simulation starts from.

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn rnd(seed: u64, id: u64, ch: u64) -> f32 {
    let h = mix(seed.wrapping_mul(0x9e3779b97f4a7c15)
        ^ mix(id.wrapping_add(ch.wrapping_mul(0x632be59bd9b4e019))));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

// ---------------------------------------------------------------------------------------------
// Simulation

#[derive(Clone, Debug, Default)]
pub struct Particle {
    pub id: u64,
    pub p: [f32; 3],
    pub v: [f32; 3],
    pub age: f32,
    pub life: f32,
    pub size: f32,
    pub opacity: f32,
    pub color: [f32; 3],
    /// 0–1 position in the gradient for Random from Gradient
    pub grad: f32,
    /// rotation in the image plane (radians, clockwise) at birth, and spin (radians per second)
    pub rot: f32,
    pub spin: f32,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub k: u64,
    pub ps: Vec<Particle>,
    /// the trails' particles
    pub aux: Vec<Particle>,
    acc: f64,
    next_id: u64,
    /// emission events so far (On a Grid: which cell is next)
    events: u64,
    /// main particles' positions before the step (scratch)
    prev: Vec<[f32; 3]>,
    /// Bounce, per main particle while the floor is on (kept out of `Particle`, which it would
    /// pad from 72 to 80 bytes): 0 free; ±k on the floor's ± side for k steps (Slide); STUCK
    contact: Vec<(i8, u8)>,
    /// Layer emitter: each main particle's birth pixel as a fraction of the layer's width and
    /// height (Layer Sampling Current Time), kept beside `ps` like `contact`
    layer_uv: Vec<[f32; 2]>,
}

fn rotate(v: [f32; 3], r: [f32; 3]) -> [f32; 3] {
    // X, then Y, then Z (the reference's orientation order for a layer's rotations)
    let (s, c) = r[0].sin_cos();
    let v = [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c];
    let (s, c) = r[1].sin_cos();
    let v = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
    let (s, c) = r[2].sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}

fn unit_sphere(u: f32, v: f32) -> [f32; 3] {
    let z = 2.0 * u - 1.0;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let a = std::f32::consts::TAU * v;
    [r * a.cos(), r * a.sin(), z]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn len3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `a + b·s`
fn mad3(a: [f32; 3], b: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

/// The Bounce floor: a plane through `at` with unit normal `n`; in it a point `at + x·u + y·w`
/// has floor coordinates (x, y) (`u`, `w` orthogonal, a layer's x and y: unit vectors for the
/// controls' floor, one image pixel of the floor layer's for a host-placed one). Layer Size: only
/// (x, y) within `rect` (x0, y0, x1, y1) collides; Layer Alpha: only where the floor layer's
/// `mask` (in the same coordinates) is set. Both sides collide (reference: a floor turned X Rotation 90
/// or −90 gives the same bounces).
#[derive(Clone, Debug)]
pub struct Floor {
    pub at: [f32; 3],
    pub n: [f32; 3],
    /// the plane at the step's start (a moving floor layer; else `at`, `n`)
    pub at0: [f32; 3],
    pub n0: [f32; 3],
    pub u: [f32; 3],
    pub w: [f32; 3],
    pub rect: Option<[f32; 4]>,
    /// Bounce Mode Layer Alpha (needs `mask`; without one it acts as Layer Size)
    pub alpha_mode: bool,
    pub mask: Option<std::sync::Arc<FloorMask>>,
    /// Collision Event: 0 Bounce, 1 Slide, 2 Stick, 3 Kill
    pub event: usize,
    pub strength: f32,
    pub random: f32,
    pub friction: f32,
    /// its layer's id (-1: none) and the layer parameter that picks it
    pub layer: i64,
    pub param: &'static str,
    /// where it is is known (the controls' floor, or placed from its layer); else it's ignored
    pub placed: bool,
}

/// The floor layer's opaque pixels (any alpha) in its image grid: pixel (i, j) covers floor
/// coordinates i..i+1, j..j+1.
#[derive(Debug)]
pub struct FloorMask {
    pub w: usize,
    pub h: usize,
    pub on: Vec<bool>,
}

impl FloorMask {
    pub fn new(px: &[Px], w: usize, h: usize) -> Self {
        FloorMask {
            w,
            h,
            on: px.par_iter().map(|p| p[3] > 0.0).collect(),
        }
    }
}

impl Floor {
    /// What a step of Slide Friction leaves of the tangential speed: friction in proportion to
    /// how hard gravity presses the particle on the plane (reference: 10 x Slide Friction per second
    /// on a floor at Gravity 500, B8 / B11; four times that at Gravity 2000, BS3 / BS5; none on
    /// a wall, BS4).
    /// a particle resting on it goes round the reference's REST_CYCLE
    fn hops(&self) -> bool {
        self.event == 0 && self.strength >= REST_HOP_STRENGTH
    }

    fn keep(&self, g: f32, dt: f32) -> f32 {
        (-self.friction * dt * (g * self.n[1]).abs() / 100.0).exp()
    }

    fn dist(&self, p: [f32; 3]) -> f32 {
        dot3(mad3(p, self.at, -1.0), self.n)
    }

    fn dist0(&self, p: [f32; 3]) -> f32 {
        dot3(mad3(p, self.at0, -1.0), self.n0)
    }

    fn inside(&self, p: [f32; 3]) -> bool {
        if self.rect.is_none() {
            return true;
        }
        let d = mad3(p, self.at, -1.0);
        let x = dot3(d, self.u) / dot3(self.u, self.u);
        let y = dot3(d, self.w) / dot3(self.w, self.w);
        if let Some(m) = self.mask.as_ref().filter(|_| self.alpha_mode) {
            let (i, j) = (x.floor(), y.floor());
            return i >= 0.0
                && j >= 0.0
                && (i as usize) < m.w
                && (j as usize) < m.h
                && m.on[j as usize * m.w + i as usize];
        }
        self.rect
            .is_none_or(|r| x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3])
    }

    /// Puts the floor where the Floor Layer is: `m` takes its image's (x, y, depth) to this
    /// layer's pixels and depth (EffectCraft's layer_transform), `rect` is its own extent in that
    /// image (None: unknown, then it collides everywhere, as Surface). The reference uses the
    /// layer's whole transform (ref BL1-BL4: its size, scale, anchor point and rotations).
    /// `m0`: where it was at the step's start (the same for a still one).
    pub fn place(&mut self, m: &[[f32; 4]; 3], m0: &[[f32; 4]; 3], rect: Option<[f32; 4]>) {
        let plane = |m: &[[f32; 4]; 3]| {
            let col = |j: usize| [m[0][j], m[1][j], m[2][j]];
            let (u, w) = (col(0), col(1));
            let c = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            let l = len3(c).max(1e-12);
            (col(3), c.map(|x| -x / l), u, w)
        };
        (self.at, self.n, self.u, self.w) = plane(m);
        (self.at0, self.n0, _, _) = plane(m0);
        self.placed = true;
        if self.rect.is_some() {
            self.rect = rect;
        }
    }
}

/// `State::contact` of a particle stuck to a bounce layer.
const STUCK: i8 = i8::MAX;

/// One step of a main particle with Bounce on (measured on reference renders, refs B5–B12):
/// - Bounce: the centre reflects off the plane at the instant it crosses, within the step; the
///   normal speed is kept at Bounce % and the tangential speed whole. Bounce Random R turns the
///   outgoing direction by R·(1 + 0.68R) radians in a random direction around it and scales its
///   speed by exp(−R·(0.7 + u)), u uniform (R = 10/30/60%: 6°/21°/48°, speed 0.83–0.92,
///   0.58–0.82, 0.37–0.68).
/// - Slide: the particle is put on the plane with its tangential travel and stays on it;
///   from its second step there the tangential speed decays at 10·Slide Friction per second
///   (Gravity 500, a floor; in proportion to gravity's push on the plane, see `Floor::keep`).
/// - A weak bounce takes a step of friction, and one too weak to lift off rests: `settle`.
/// - Stick: it stops where it crossed. Kill: it dies.
/// - A moving floor (layer) is met where it is during the step, moving evenly from where it was
///   at the start to where it is at the end; its own motion adds nothing to the bounce (ref BL6).
///   Tested against a still plane, a floor rising past a particle that just bounced off it
///   would knock it back through from below.
/// - Several bounce layers (the reference's three, each with its own settings; ref BW1-BW5, BX1-BX3:
///   a wall bounces like a floor, Bounce 2 and 3 alike): the one crossed first takes the step;
///   the rest of the step after a bounce, and a sliding particle's step, are checked against the
///   others (a corner, a wall across the floor it slides on).
///
/// `c`: 0 free, ±k sliding for k steps on the ± side of plane `c.1`, or STUCK.
#[allow(clippy::too_many_arguments)]
fn floor_step(
    q: &mut Particle,
    c: &mut (i8, u8),
    b: &Params,
    fls: &[Floor],
    g: f32,
    t1: f32,
    dt: f32,
    k: u64,
) {
    if c.0 == STUCK {
        q.age += dt;
        return;
    }
    let slid = (c.0 != 0)
        .then_some(c.1 as usize)
        .filter(|&s| fls.get(s).is_some_and(|f| f.placed));
    if slid.is_none() {
        c.0 = 0;
    }
    // friction: Slide from the second step on the plane; a Bounce layer's resting particle
    // on the first step of each REST_CYCLE
    if let Some(s) = slid.filter(|&s| {
        let n = c.0.unsigned_abs();
        if fls[s].event == 1 {
            n >= 2
        } else {
            n == 1 && fls[s].hops()
        }
    }) {
        let keep = fls[s].keep(g, dt);
        q.v = q.v.map(|x| x * keep);
    }
    let p0 = q.p;
    advance(q, b, b.turbulence, b.air, g, t1, dt);
    if let Some(s) = slid {
        // sliding: leaves the floor when it moves off it or over its edge
        let fl = &fls[s];
        let d1 = fl.dist(q.p);
        let side = c.0.signum() as f32;
        if d1 * side > 0.0 || !fl.inside(q.p) {
            c.0 = 0;
        } else {
            q.v = mad3(q.v, fl.n, -dot3(q.v, fl.n));
            let n = if fl.hops() {
                c.0.abs() % REST_CYCLE.len() as i8 + 1
            } else {
                (c.0.abs() + 1).min(100)
            };
            let hop = if fl.hops() {
                REST_CYCLE[n as usize - 1] * (g * fl.n[1]).abs() * dt * dt
            } else {
                0.0
            };
            q.p = mad3(q.p, fl.n, side * hop - d1);
            c.0 = n * c.0.signum();
        }
        hit_others(q, c, p0, fls, s, b, k, g, dt);
        return;
    }
    // the first crossing on the step's path: p(t) = p0 + v·t + ½·g·t² ŷ with v the velocity
    // after drag
    let vd = mad3(q.v, [0.0, 1.0, 0.0], -g * dt);
    let mut hit: Option<(f32, usize, f32, f32, [f32; 3])> = None;
    for (j, fl) in fls.iter().enumerate().filter(|(_, f)| f.placed) {
        let d0 = fl.dist0(p0);
        let d1 = fl.dist(q.p);
        if d0 == 0.0 || d0 * d1 > 0.0 {
            continue;
        }
        // how far the floor moved towards p0 over the step (0 for a still one)
        let drift = fl.dist(p0) - d0;
        let (qa, qb) = (0.5 * g * fl.n[1], dot3(vd, fl.n) + drift / dt);
        let lin = d0 / (d0 - d1) * dt;
        let tc = if qa.abs() < 1e-6 {
            lin
        } else {
            let disc = qb * qb - 4.0 * qa * d0;
            let roots = if disc >= 0.0 {
                let s = disc.sqrt();
                [(-qb - s) / (2.0 * qa), (-qb + s) / (2.0 * qa)]
            } else {
                [lin; 2]
            };
            roots
                .into_iter()
                .filter(|t| (0.0..=dt * 1.0001).contains(t))
                .fold(f32::INFINITY, f32::min)
                .min(dt)
        };
        if hit.is_some_and(|h| h.0 <= tc) {
            continue;
        }
        let pc = mad3(mad3(p0, vd, tc), [0.0, 0.5 * g * tc * tc, 0.0], 1.0);
        if fl.inside(pc) {
            hit = Some((tc, j, d0, d1, pc));
        }
    }
    let Some((tc, j, d0, d1, pc)) = hit else {
        return;
    };
    let fl = &fls[j];
    let side = d0.signum();
    // the velocity at the crossing
    let vc = mad3(vd, [0.0, 1.0, 0.0], g * tc);
    match fl.event {
        3 => q.life = -1.0,
        2 => {
            q.p = pc;
            q.v = [0.0; 3];
            *c = (STUCK, j as u8);
        }
        1 => {
            // The reference keeps the velocity it crossed at (BS4: sliding down a wall, no gravity for the
            // rest of that step)
            q.p = mad3(q.p, fl.n, -d1);
            q.v = mad3(vc, fl.n, -dot3(vc, fl.n));
            *c = (side as i8, j as u8);
        }
        _ => {
            let v = bounced(vc, fl, side, b, q.id, k);
            let out = dot3(v, fl.n) * side;
            // the rest of the step from the floor
            let tr = dt - tc;
            q.p = pc;
            q.v = v;
            advance(q, b, b.turbulence, b.air, g, t1, tr);
            // The reference moves it the rest of the step by Euler (velocity first), so gravity takes it
            // g·tr² down, not ½·g·tr² (refs BS3, BS15: bounces early in a step land low)
            q.p[1] += 0.5 * g * tr * tr;
            q.age -= tr;
            let d = fl.dist(q.p);
            if d * side < 0.0 {
                // just off it, on its side: exactly on it, the next step couldn't tell the side
                // (a floor rising under a resting particle would let it through)
                q.p = mad3(q.p, fl.n, side * 1e-3 - d);
                let vn = dot3(q.v, fl.n);
                if vn * side < 0.0 {
                    q.v = mad3(q.v, fl.n, -vn);
                }
            }
            settle(q, c, fl, j, side, out, g, dt, tc / dt);
            hit_others(q, c, pc, fls, j, b, k, g, dt);
        }
    }
}

/// A Bounce that leaves at `out` (normal speed away from the plane) settles as in the reference (refs
/// BW9, BS1-BS15, Gravity 500-3000, Bounce 20-70%): one leaving under 0.75·g_n·dt comes to rest
/// on the plane (BS11: 84 px/s at g_n·dt = 133 does; BS12: 49 px/s at 40 doesn't), in the reference's
/// REST_CYCLE, until it moves off the plane or over its edge, and takes a step of Slide Friction
/// off its tangential speed; so does one leaving under WEAK_BOUNCE px/s that bounced in the
/// first half of its step (`tcf`, the crossing's time in the step; BW9: 42 px/s at 0.05 does,
/// BS12: 49 px/s at 0.83 doesn't).
#[allow(clippy::too_many_arguments)]
fn settle(
    q: &mut Particle,
    c: &mut (i8, u8),
    fl: &Floor,
    j: usize,
    side: f32,
    out: f32,
    g: f32,
    dt: f32,
    tcf: f32,
) {
    if q.life < 0.0 {
        return;
    }
    let rest = out < 0.75 * (g * fl.n[1]).abs() * dt;
    if rest || (out < WEAK_BOUNCE && tcf <= 0.5) {
        let vn = dot3(q.v, fl.n);
        let vt = mad3(q.v, fl.n, -vn);
        let keep = fl.keep(g, dt);
        q.v = mad3(vt.map(|x| x * keep), fl.n, vn);
    }
    if rest {
        q.p = mad3(q.p, fl.n, side * 1e-3 - fl.dist(q.p));
        q.v = mad3(q.v, fl.n, -dot3(q.v, fl.n));
        *c = (side as i8, j as u8);
    }
}

/// A particle resting on a Bounce layer at Bounce >= REST_HOP_STRENGTH (refs BS3, BS6-BS9,
/// BS14, Gravity 750-3000 alike in units of g_n·dt²): The reference never lets it lie still. Every 4 steps
/// it takes a step of Slide Friction (the first step after it lands), then hops off the plane
/// and lands again, so it comes to a stop. Its
/// height over the plane after each step, in g_n·dt² (the reference's steady cycle), from the step that
/// lands it: resting, friction, hop, landing.
/// ponytail: The reference's steady Bounce 50% cycle for every Bounce; the reference's hops vary a little with Bounce
/// (45%: 0.374, 0.0315) and settle in over the first few cycles.
const REST_CYCLE: [f32; 4] = [0.0, 0.0, 0.432, 0.0277];

/// Bounce at which a resting particle keeps hopping (reference: 45% and up do; at 40% the hops die
/// out after 4 cycles, when it has all but stopped, BS13; 20% and 35% lie still, BS11, BS12)
const REST_HOP_STRENGTH: f32 = 0.375;

/// A bounce leaving slower than this (px/s) early in its step takes a step of friction (reference: 42
/// and 44 px/s do, 110 px/s and faster don't).
const WEAK_BOUNCE: f32 = 75.0;

/// The velocity `vc` bounced off `fl` from its `side`: the normal part kept at Bounce %, turned
/// and slowed by Bounce Random.
fn bounced(vc: [f32; 3], fl: &Floor, side: f32, b: &Params, id: u64, k: u64) -> [f32; 3] {
    let vn = dot3(vc, fl.n);
    let mut v = mad3(vc, fl.n, -(1.0 + fl.strength) * vn);
    if fl.random > 0.0 {
        let r = fl.random;
        let s = b.seed ^ k.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0xb0b0;
        let (az, ru) = (std::f32::consts::TAU * rnd(s, id, 1), rnd(s, id, 2));
        let sp = len3(v);
        if sp > 0.0 {
            let d = v.map(|x| x / sp);
            // two unit vectors perpendicular to d
            let a = if d[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let e1 = mad3(a, d, -dot3(a, d));
            let e1 = e1.map(|x| x / len3(e1));
            let e2 = [
                d[1] * e1[2] - d[2] * e1[1],
                d[2] * e1[0] - d[0] * e1[2],
                d[0] * e1[1] - d[1] * e1[0],
            ];
            let th = r * (1.0 + 0.68 * r);
            let (st, ct) = th.sin_cos();
            let mut nd = mad3(
                mad3(d.map(|x| x * ct), e1, st * az.cos()),
                e2,
                st * az.sin(),
            );
            // never into the floor
            let dn = dot3(nd, fl.n);
            if dn * side < 0.0 {
                nd = mad3(nd, fl.n, -2.0 * dn);
            }
            let m = sp * (-r * (0.7 + ru)).exp();
            v = nd.map(|x| x * m);
        }
    }
    v
}

/// The rest of a step, from `from` to `q.p` (nearly straight), checked against the bounce
/// layers other than `skip`: the first one crossed takes its Collision Event there (Bounce:
/// the overshoot mirrored back at Bounce %).
#[allow(clippy::too_many_arguments)]
fn hit_others(
    q: &mut Particle,
    c: &mut (i8, u8),
    from: [f32; 3],
    fls: &[Floor],
    skip: usize,
    b: &Params,
    k: u64,
    g: f32,
    dt: f32,
) {
    if fls.len() < 2 || q.life < 0.0 {
        return;
    }
    let mut first: Option<(f32, usize, f32)> = None;
    for (j, fl) in fls.iter().enumerate() {
        if j == skip || !fl.placed {
            continue;
        }
        let (a, e) = (fl.dist(from), fl.dist(q.p));
        if a == 0.0 || a * e > 0.0 {
            continue;
        }
        let t = a / (a - e);
        if first.is_none_or(|f| t < f.0) && fl.inside(lerp3(from, q.p, t)) {
            first = Some((t, j, a));
        }
    }
    let Some((t, j, a)) = first else {
        return;
    };
    let fl = &fls[j];
    let (side, e) = (a.signum(), fl.dist(q.p));
    match fl.event {
        3 => q.life = -1.0,
        2 => {
            q.p = lerp3(from, q.p, t);
            q.v = [0.0; 3];
            *c = (STUCK, j as u8);
        }
        1 => {
            q.p = mad3(q.p, fl.n, -e);
            q.v = mad3(q.v, fl.n, -dot3(q.v, fl.n));
            *c = (side as i8, j as u8);
        }
        _ => {
            q.v = bounced(q.v, fl, side, b, q.id, k);
            q.p = mad3(q.p, fl.n, -e * (1.0 + fl.strength));
            let d = fl.dist(q.p);
            if d * side < 1e-3 {
                q.p = mad3(q.p, fl.n, side * 1e-3 - d);
            }
            let out = dot3(q.v, fl.n) * side;
            settle(q, c, fl, j, side, out, g, dt, 0.0);
        }
    }
}

/// Gravity in px/s² per unit (measured: Gravity 100 falls 100 px in the first second).
const GRAVITY: f32 = 2.0;
/// Air resistance is quadratic in the speed relative to the wind (fitted on two drag rates):
/// dv/dt = -k (DRAG_LIN + DRAG_QUAD |v - w|) (v - w).
const DRAG_LIN: f32 = 0.04;
const DRAG_QUAD: f32 = 0.0028;
/// Air turbulence is a wind field (it acts through the air resistance, as in the reference): speed
/// in px/s per unit of Affect Position, its octaves summed then divided by their total weight,
/// and a noise cell of TURB_CELL / Scale px: a bigger Scale gives finer turbulence. Fitted
/// (0.17.6) on the reference's field over a 40 x 22 grid of still particles (TB1, TB2: Scale 10 and 20, one
/// octave: pushed 6.65 and 6.6 px in 5 frames vs the reference 6.4 and 6.2; correlation 45 / 90 px apart
/// 0.57 / 0.04 and 0.09 / -0.07 vs the reference 0.56 / 0.10 and 0.08 / -0.02) and a 300 px/s stream at
/// Affect Position 100, three octaves (S11: reach 510-662 px over three seeds vs the reference 524, vertical
/// spread 129-271 px vs 179). The noise pattern itself isn't the reference's.
const TURB_SPEED: f32 = 10.7;
const TURB_CELL: f32 = 1400.0;

pub fn curve(kind: usize, u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    match kind {
        1 => 1.0 - u,
        2 => u,
        3 => 1.0 - (2.0 * u - 1.0).abs(),
        4 => (u / 0.1).min((1.0 - u) / 0.1).min(1.0),
        _ => 1.0,
    }
}

/// The emitter layer for a step: its pixels in a buffer grid (`origin`, `scale` map buffer to
/// layer pixels like the frame's), the indices of its opaque ones (see [`opaque_pixels`]) and
/// the layer's centre in layer pixels. EffectCraft hands a layer over in its own space (its
/// content, effects and masks, not its transform), so the layer's centre is put at Position:
/// at the default, the comp's centre, a comp-sized layer lands where it is; animate Position
/// (or link it to the layer's position with an expression) to move the emitter.
pub struct EmitLayer<'a> {
    pub px: &'a [Px],
    pub w: usize,
    pub h: usize,
    pub origin: [f32; 2],
    pub scale: f32,
    pub opaque: &'a [u32],
    pub center: [f32; 2],
    /// the layer's transform at the step's start and end, when EffectCraft gives it: image
    /// x, y and depth z → this layer's pixels and depth (rows of a 3 × 4 matrix)
    pub xf: Option<[[[f32; 4]; 3]; 2]>,
}

/// The indices of `px`'s pixels with any alpha (ref emits from every pixel above zero alpha,
/// as many from a 50% one as from an opaque one); with `edges`, only those where the alpha
/// crosses 50% (one of them or a 4-neighbour at least half opaque, another under half): the
/// outline of a text or a mask, about two pixels wide. `w`: the image's width.
pub fn opaque_pixels(px: &[Px], w: usize, edges: bool, out: &mut Vec<u32>) {
    let alpha = |i: isize| -> f32 {
        if i < 0 || i as usize >= px.len() {
            0.0
        } else {
            px[i as usize][3]
        }
    };
    let edge = |i: usize, a: f32| {
        let (x, ii, wi) = (i % w.max(1), i as isize, w as isize);
        let n = [
            if x > 0 { alpha(ii - 1) } else { 0.0 },
            if x + 1 < w { alpha(ii + 1) } else { 0.0 },
            alpha(ii - wi),
            alpha(ii + wi),
        ];
        let lo = n.iter().fold(a, |m, &v| m.min(v));
        let hi = n.iter().fold(a, |m, &v| m.max(v));
        lo < 0.5 && hi >= 0.5
    };
    // in parallel: the image was just written by EffectCraft, so reading it page-faults
    const CH: usize = 1 << 16;
    let parts: Vec<Vec<u32>> = px
        .par_chunks(CH)
        .enumerate()
        .map(|(c, ch)| {
            ch.iter()
                .enumerate()
                .filter(|&(i, p)| p[3] > 1e-3 && (!edges || edge(c * CH + i, p[3])))
                .map(|(i, _)| (c * CH + i) as u32)
                .collect()
        })
        .collect();
    out.clear();
    for p in parts {
        out.extend(p);
    }
}

/// The emitter layer's rotation (comp layer X / Y / Z Rotation, applied Z first, then Y, then X:
/// refs LR8_*) times its scale.
fn layer_matrix([x, y, z]: [f32; 3], k: f32) -> [[f32; 3]; 3] {
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    let mul = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| {
        let mut o = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                o[i][j] = (0..3).map(|n| a[i][n] * b[n][j]).sum();
            }
        }
        o
    };
    mul(mul(rx, ry), rz).map(|row| row.map(|v| v * k))
}

/// Layer Sampling Current Time: each main particle's colour in the layer image `px` (`w` × `h`)
/// at its birth pixel, [-1, ..] where it has none (born elsewhere, or there's no alpha now).
pub fn layer_colors(st: &State, px: &[Px], w: usize, h: usize, out: &mut Vec<[f32; 3]>) {
    out.clear();
    if st.layer_uv.len() != st.ps.len() || w == 0 || h == 0 {
        return;
    }
    out.par_extend(st.layer_uv.par_iter().map(|&[u, v]| {
        if u < 0.0 {
            return [-1.0; 3];
        }
        let p = px[((v * h as f32) as usize).min(h - 1) * w + ((u * w as f32) as usize).min(w - 1)];
        if p[3] > 1e-3 {
            [p[0], p[1], p[2]].map(|c| c / p[3])
        } else {
            [-1.0; 3]
        }
    }));
}

/// Rotation (radians, clockwise) from a 0-1 layer value: none at 0.5, the reference ±44.2°, 86.7°, 127.5°,
/// 167° at 0.625 / 0.75 / 0.875 / 1 and the mirror below (refs LG6, LG8).
fn layer_turn(v: f32) -> f32 {
    let u = 2.0 * v - 1.0;
    (180.0 * u - 13.2 * u * u.abs()).to_radians()
}

/// A light the Light(s) emitter emits from: where it is (layer pixels, z away from the viewer)
/// and its Intensity (1 = 100%).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmitLight {
    pub pos: [f32; 3],
    pub intensity: f32,
}

/// A 3D model for the 3D Model emitter: its vertices fitted into [-0.5, 0.5]^3 (the longest
/// side spans it, centred; y and z turned to the comp's: y down, z away), its triangles and
/// edges, with running totals of their areas and lengths for picking them by size.
#[derive(Debug, Default)]
pub struct Mesh {
    pub v: Vec<[f32; 3]>,
    pub tri: Vec<[u32; 3]>,
    pub edges: Vec<[u32; 2]>,
    area: Vec<f32>,
    length: Vec<f32>,
}

impl Mesh {
    /// Reads the `v` and `f` lines of an OBJ file (polygons as fans, negative indices from the
    /// end); None when it has no vertices.
    pub fn from_obj(text: &str) -> Option<Mesh> {
        let mut v: Vec<[f32; 3]> = Vec::new();
        let mut tri = Vec::new();
        // the polygons' own edges (not the fans' diagonals)
        let mut edges: Vec<[u32; 2]> = Vec::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            match it.next() {
                Some("v") => {
                    let c: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                    if c.len() == 3 {
                        v.push([c[0], -c[1], -c[2]]);
                    }
                }
                Some("f") => {
                    let ix: Vec<u32> = it
                        .filter_map(|x| x.split('/').next()?.parse::<i64>().ok())
                        .filter_map(|i| {
                            let i = if i < 0 { v.len() as i64 + i } else { i - 1 };
                            (0..v.len() as i64).contains(&i).then_some(i as u32)
                        })
                        .collect();
                    for k in 1..ix.len().saturating_sub(1) {
                        tri.push([ix[0], ix[k], ix[k + 1]]);
                    }
                    for k in 0..ix.len() {
                        let (a, b) = (ix[k], ix[(k + 1) % ix.len()]);
                        if ix.len() > 1 && a != b {
                            edges.push([a.min(b), a.max(b)]);
                        }
                    }
                }
                _ => {}
            }
        }
        if v.is_empty() {
            return None;
        }
        // fit into the unit cube, centred
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &v {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let k = 1.0 / (0..3).map(|i| hi[i] - lo[i]).fold(1e-9f32, f32::max);
        for p in &mut v {
            for i in 0..3 {
                p[i] = (p[i] - 0.5 * (lo[i] + hi[i])) * k;
            }
        }
        edges.sort_unstable();
        edges.dedup();
        let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let mut acc = 0.0;
        let area = tri
            .iter()
            .map(|t| {
                let (a, b) = (
                    sub(v[t[1] as usize], v[t[0] as usize]),
                    sub(v[t[2] as usize], v[t[0] as usize]),
                );
                acc += 0.5 * len3(cross(a, b));
                acc
            })
            .collect();
        let mut acc = 0.0;
        let length = edges
            .iter()
            .map(|e| {
                acc += len3(sub(v[e[1] as usize], v[e[0] as usize]));
                acc
            })
            .collect();
        Some(Mesh {
            v,
            tri,
            edges,
            area,
            length,
        })
    }

    /// A random point on it from three uniform numbers: a vertex (0), a point on an edge picked
    /// by length (1), or on a face picked by area (2); its vertices when it has no edges or faces.
    pub fn sample(&self, from: usize, a: f32, b: f32, c: f32) -> [f32; 3] {
        let pick = |tot: &[f32], u: f32| {
            let x = u * tot.last().copied().unwrap_or(0.0);
            tot.partition_point(|&t| t < x).min(tot.len() - 1)
        };
        let lerp = |p: [f32; 3], q: [f32; 3], t: f32| lerp3(p, q, t);
        match from {
            1 if !self.edges.is_empty() => {
                let e = self.edges[pick(&self.length, a)];
                lerp(self.v[e[0] as usize], self.v[e[1] as usize], b)
            }
            2 if !self.tri.is_empty() => {
                let t = self.tri[pick(&self.area, a)];
                // uniform on the triangle
                let s = b.sqrt();
                let (w0, w1, w2) = (1.0 - s, s * (1.0 - c), s * c);
                let (p, q, r) = (
                    self.v[t[0] as usize],
                    self.v[t[1] as usize],
                    self.v[t[2] as usize],
                );
                std::array::from_fn(|i| p[i] * w0 + q[i] * w1 + r[i] * w2)
            }
            _ => self.v[((a * self.v.len() as f32) as usize).min(self.v.len() - 1)],
        }
    }
}

/// Lighting's settings: Ambient and Diffuse (1 = 100%), and the Nominal Distance of Natural
/// falloff (None: lights don't fall off).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    pub ambient: f32,
    pub diffuse: f32,
    pub nominal: Option<f32>,
}

/// Self-Shadow's settings (fractions: 1 = 100%).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelfShadow {
    pub softness: f32,
    pub color: [f32; 3],
    pub strength: f32,
    pub opacity: f32,
    pub size: f32,
    pub distance: f32,
}

/// A self-shadow's radius and its offset away from the light, in particle radii at Adjust Size and
/// Adjust Distance 100% (fitted on the reference's clouds SH5-SH11).
const SHADOW_SIZE: f32 = 2.5;
const SHADOW_DIST: f32 = 1.5;

/// A comp light as Lighting uses it: kind (0 parallel, 1 spot, 2 point, 3 ambient), position
/// (layer pixels, z away from the viewer), the unit direction it shines, colour times
/// Intensity, and a spot's cone: half angle and feather (0-1 of the half angle), in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadeLight {
    pub kind: u8,
    pub pos: [f32; 3],
    pub dir: [f32; 3],
    pub color: [f32; 3],
    pub half: f32,
    pub feather: f32,
}

/// The light a particle at `p` gets from the comp's lights, per channel (ref LS1-LS16, measured
/// on Spheres): an ambient light gives Ambient x its colour, a point light Diffuse x its colour x
/// (Nominal Distance / distance)^2 with Natural falloff (no falloff with None, nor with the
/// light's own falloff settings), a spot the same inside its cone, faded by a smoothstep over the
/// feathered edge, with no angle to the light. Every particle type gets the same (the reference
/// tilts a sprite's light by its angle to the light, LS21, not copied); a parallel light gives
/// Diffuse x its colour everywhere (the reference's parallel lights light nothing, LS7, LS12, LS13,
/// not copied).
pub fn shade(lt: &Lighting, lights: &[ShadeLight], p: [f32; 3]) -> [f32; 3] {
    let mut acc = [0.0f32; 3];
    for l in lights {
        let k = match l.kind {
            3 => lt.ambient,
            0 => lt.diffuse,
            _ => {
                let d = [p[0] - l.pos[0], p[1] - l.pos[1], p[2] - l.pos[2]];
                let dist = len3(d).max(1e-3);
                let mut k = lt.diffuse;
                if let Some(n) = lt.nominal {
                    k *= (n / dist) * (n / dist);
                }
                if l.kind == 1 {
                    let c = (d[0] * l.dir[0] + d[1] * l.dir[1] + d[2] * l.dir[2]) / dist;
                    let ang = c.clamp(-1.0, 1.0).acos();
                    let inner = l.half * (1.0 - l.feather);
                    let t = if l.half - inner > 1e-6 {
                        ((l.half - ang) / (l.half - inner)).clamp(0.0, 1.0)
                    } else if ang <= l.half {
                        1.0
                    } else {
                        0.0
                    };
                    k *= t * t * (3.0 - 2.0 * t);
                }
                k
            }
        };
        for i in 0..3 {
            acc[i] += k * l.color[i];
        }
    }
    acc
}

/// A point of the cube [-1, 1]^3 mapped onto the unit ball, spreading the cube evenly through
/// it (the usual cube-to-sphere map: faces to the sphere, the centre staying put).
fn cube_to_ball(p: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = p;
    let (x2, y2, z2) = (x * x, y * y, z * z);
    [
        x * (1.0 - y2 / 2.0 - z2 / 2.0 + y2 * z2 / 3.0).max(0.0).sqrt(),
        y * (1.0 - z2 / 2.0 - x2 / 2.0 + z2 * x2 / 3.0).max(0.0).sqrt(),
        z * (1.0 - x2 / 2.0 - y2 / 2.0 + x2 * y2 / 3.0).max(0.0).sqrt(),
    ]
}

/// The emitter's Position at `frac` of the step before its end, and its velocity there: a
/// straight line from a to b, or through `b.sub_pos` (Position Subframe), joined straight
/// (10x Linear) or by a cubic Hermite curve (10x Smooth). The velocity is the curve's in both
/// (straight pieces' own velocities would part the particles at every sample). Its tangents are
/// central differences of the samples (Catmull-Rom) and, at a frame, a backward second-order
/// difference that the steps on either side share (the step after reads the step before's
/// samples in `a.sub_pos`), so the velocity particles inherit doesn't jump at frames.
pub fn emitter_path(a: &Params, b: &Params, frac: f32, dt: f32) -> ([f32; 3], [f32; 3]) {
    if b.sub_pos.is_empty() {
        let v = std::array::from_fn(|i| (b.pos[i] - a.pos[i]) / dt);
        return (lerp3(b.pos, a.pos, frac), v);
    }
    let n = b.sub_pos.len() + 1;
    let pt = |j: usize| match j {
        0 => a.pos,
        j if j >= n => b.pos,
        j => b.sub_pos[j - 1],
    };
    let t = (1.0 - frac).clamp(0.0, 1.0) * n as f32;
    let j = (t as usize).min(n - 1);
    let u = t - j as f32;
    let h = dt / n as f32;
    let (p1, p2) = (pt(j), pt(j + 1));
    // tangent (px/s) at sample j
    let back = |q: [f32; 3], q1: [f32; 3], q2: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|i| (3.0 * q[i] - 4.0 * q1[i] + q2[i]) / (2.0 * h))
    };
    let tan = |j: usize| -> [f32; 3] {
        if j >= n {
            back(pt(n), pt(n - 1), pt(n - 2))
        } else if j > 0 {
            let (q0, q2) = (pt(j - 1), pt(j + 1));
            std::array::from_fn(|i| (q2[i] - q0[i]) / (2.0 * h))
        } else if a.sub_pos.len() == n - 1 {
            back(a.pos, a.sub_pos[n - 2], a.sub_pos[n - 3])
        } else {
            let (q1, q2) = (pt(1), pt(2));
            std::array::from_fn(|i| (-3.0 * a.pos[i] + 4.0 * q1[i] - q2[i]) / (2.0 * h))
        }
    };
    let (m1, m2) = (tan(j), tan(j + 1));
    let (u2, u3) = (u * u, u * u * u);
    let (h00, h10, h01, h11) = (
        2.0 * u3 - 3.0 * u2 + 1.0,
        u3 - 2.0 * u2 + u,
        3.0 * u2 - 2.0 * u3,
        u3 - u2,
    );
    let (d00, d10, d11) = (
        6.0 * u2 - 6.0 * u,
        3.0 * u2 - 4.0 * u + 1.0,
        3.0 * u2 - 2.0 * u,
    );
    let p = std::array::from_fn(|i| h00 * p1[i] + h10 * h * m1[i] + h01 * p2[i] + h11 * h * m2[i]);
    let v = std::array::from_fn(|i| d00 * (p1[i] - p2[i]) / h + d10 * m1[i] + d11 * m2[i]);
    if b.subframe < 2 {
        return (lerp3(p1, p2, u), v);
    }
    (p, v)
}

/// Emits one particle per birth: (fraction of the step it was born before its end, On a Grid
/// cell or u32::MAX).
fn emit(
    st: &mut State,
    a: &Params,
    b: &Params,
    dt: f32,
    births: &[(f32, u32)],
    layer: Option<&EmitLayer>,
) {
    for &(frac, cell) in births {
        let (epos, evel) = emitter_path(a, b, frac, dt);
        let id = st.next_id;
        st.next_id += 1;
        let s = b.seed;
        let r = |ch| rnd(s, id, ch);
        // Ages spread over the frame, the newest at 0; the emitter moves along between frames.
        // An explosion spreads its particles' ages over the first frame the same way (reference: one
        // particle starts at the emitter on frame 1, refs B13-B16; 50 at 300 px/s are 0-10 px
        // out on frame 1 and up to 301 px at 1 s, S0b / S0). Before 0.7.0 all started half a
        // frame old.
        let age = frac * dt;
        // Light(s): one of the lights, by Intensity
        let light = (b.emitter_type == 4 && !b.lights.is_empty())
            .then(|| {
                let total: f32 = b.lights.iter().map(|l| l.intensity.max(0.0)).sum();
                let mut x = r(19) * total;
                b.lights.iter().position(|l| {
                    x -= l.intensity.max(0.0);
                    x < 0.0
                })
            })
            .flatten()
            .or((b.emitter_type == 4 && !b.lights.is_empty()).then(|| b.lights.len() - 1));
        // where that light was at the birth: along its way from the step's start (the lights come
        // in the same order each step), so a moving light lays an even trail, not a clump a frame
        let base = match light {
            Some(i) if a.lights.len() == b.lights.len() => {
                lerp3(b.lights[i].pos, a.lights[i].pos, frac)
            }
            Some(i) => b.lights[i].pos,
            None => epos,
        };
        // On a Grid, Box and Sphere: the cell centre, (i + 0.5) / n of each size (ref GB1-GB5,
        // as the Layer grid; the reference lays Particles in X / Size X along Z and Z's along X,
        // not copied), the Sphere's cube of cells mapped onto the ball (cube_to_ball)
        let cell_at = |n: [u32; 3]| {
            let ijk = [cell % n[0], cell / n[0] % n[1], cell / (n[0] * n[1])];
            [0, 1, 2].map(|a| 2.0 * (ijk[a] as f32 + 0.5) / n[a] as f32 - 1.0)
        };
        let grid_cell = b
            .grid
            .filter(|_| cell != u32::MAX && matches!(b.emitter_type, 1 | 2));
        let local = match b.emitter_type {
            1 | 2 if grid_cell.is_some() => {
                let u = cell_at(grid_cell.unwrap().0);
                let u = if b.emitter_type == 2 {
                    cube_to_ball(u)
                } else {
                    u
                };
                [
                    0.5 * u[0] * b.esize[0],
                    0.5 * u[1] * b.esize[1],
                    0.5 * u[2] * b.esize[2],
                ]
            }
            1 => [
                (r(1) - 0.5) * b.esize[0],
                (r(2) - 0.5) * b.esize[1],
                (r(3) - 0.5) * b.esize[2],
            ],
            // Light(s): a Sphere of Emitter Size around each light (ref LE1-LE3: x 411-790 around a
            // light at 600 with Size 500, as the Sphere emitter's spread)
            2 | 4 => {
                let d = unit_sphere(r(1), r(2));
                let k = r(3).cbrt() * 0.5;
                [
                    d[0] * k * b.esize[0],
                    d[1] * k * b.esize[1],
                    d[2] * k * b.esize[2],
                ]
            }
            // 3D Model: a point on it (fitted into the unit cube), times Emitter Size
            6 => match &b.model {
                Some(m) => {
                    let u = m.sample(b.model_from, r(1), r(2), r(3));
                    [u[0] * b.esize[0], u[1] * b.esize[1], u[2] * b.esize[2]]
                }
                None => [0.0; 3],
            },
            _ => [0.0; 3],
        };
        let mut off = rotate(local, b.rot);
        let mut p0 = [base[0] + off[0], base[1] + off[1], base[2] + off[2]];
        // Layer: a random opaque pixel, a random point in it, uniformly 0..Emitter Size Z behind
        // the layer (refs L1-L8: z quantiles 0/50/100% = 0, 266, 495 at Size Z 500); its
        // colour and alpha colour the particle
        let mut pixel = None;
        let mut uv = [-1.0f32; 2];
        // the layer point: image x, y and depth z in the layer's own pixels
        let mut lp = None;
        if let (Some(l), Some(([nx, ny, nz], _))) = (layer.filter(|_| cell != u32::MAX), b.grid) {
            // On a Grid: the cell's centre (ref LG1: an 8 × 4 grid at (i + 0.5) / 8 of the width,
            // (j + 0.5) / 4 of the height), its Z plane centred on the layer (LG14: 3 planes over
            // Emitter Size Z 600 at -200, 0, 200)
            let (i, j, k) = (cell % nx, cell / nx % ny, cell / (nx * ny));
            let bx = (i as f32 + 0.5) / nx as f32 * l.w as f32;
            let by = (j as f32 + 0.5) / ny as f32 * l.h as f32;
            let px = l.px[(by as usize).min(l.h - 1) * l.w + (bx as usize).min(l.w - 1)];
            if px[3] <= 1e-3 {
                continue;
            }
            p0 = [
                base[0] + (bx - l.origin[0]) / l.scale - l.center[0],
                base[1] + (by - l.origin[1]) / l.scale - l.center[1],
                base[2] + ((k as f32 + 0.5) / nz as f32 - 0.5) * b.esize[2],
            ];
            off = [p0[0] - base[0], p0[1] - base[1], p0[2] - base[2]];
            pixel = Some(px);
            uv = [bx / l.w as f32, by / l.h as f32];
            lp = Some([bx, by, ((k as f32 + 0.5) / nz as f32 - 0.5) * b.esize[2]]);
        } else if let Some(l) = layer.filter(|_| b.emitter_type == 3) {
            let i = l.opaque[((r(1) * l.opaque.len() as f32) as usize).min(l.opaque.len() - 1)]
                as usize;
            let bx = (i % l.w) as f32 + r(2);
            let by = (i / l.w) as f32 + r(3);
            p0 = [
                base[0] + (bx - l.origin[0]) / l.scale - l.center[0],
                base[1] + (by - l.origin[1]) / l.scale - l.center[1],
                base[2] + r(18) * b.esize[2],
            ];
            off = [p0[0] - base[0], p0[1] - base[1], p0[2] - base[2]];
            pixel = Some(l.px[i]);
            uv = [bx / l.w as f32, by / l.h as f32];
            lp = Some([bx, by, p0[2] - base[2]]);
        }
        // The emitter layer's own transform from EffectCraft (hosts listing "layer_transform"):
        // where the layer is drawn, at the particle's moment within the step. Position and the
        // Layer Rotation / Scale controls don't apply then (the reference ignores Position too).
        let host_xf = layer.and_then(|l| l.xf).zip(lp);
        if let Some(([ma, mb], u)) = host_xf {
            let at = |m: &[[f32; 4]; 3]| m.map(|w| w[0] * u[0] + w[1] * u[1] + w[2] * u[2] + w[3]);
            let (e, s0) = (at(&mb), at(&ma));
            p0 = [
                e[0] + (s0[0] - e[0]) * frac,
                e[1] + (s0[1] - e[1]) * frac,
                e[2] + (s0[2] - e[2]) * frac,
            ];
            off = [p0[0] - base[0], p0[1] - base[1], p0[2] - base[2]];
        }
        // The layer's rotation and scale, about its centre (where the layer itself turns with
        // its anchor point at the centre). The reference turns it about the layer's bottom-right
        // corner instead (ref LR1-LR8, LT1-LT5: centre + m·(d - centre)), which puts the
        // particles off the layer as it's seen; not copied, at the user's choice. The emission
        // direction doesn't turn with the layer (ref LT6).
        if let Some(m) = b
            .layer_xf
            .as_ref()
            .filter(|_| pixel.is_some() && host_xf.is_none())
        {
            off = m.map(|row| row[0] * off[0] + row[1] * off[1] + row[2] * off[2]);
            p0 = [base[0] + off[0], base[1] + off[1], base[2] + off[2]];
        }
        let dir = match b.direction {
            1 | 2 => {
                // (1 - s)·axis + s·(random unit vector), normalized: the axis is +z (away from
                // the camera); 20% reaches 14.7°, 50% and up the whole hemisphere and beyond,
                // denser towards the edge (ref end-on cones at 20% and 60%).
                let u = unit_sphere(r(4), r(5));
                let s = b.spread;
                let mut d = [s * u[0], s * u[1], (1.0 - s) + s * u[2]];
                let l = len3(d).max(1e-6);
                d = d.map(|x| x / l);
                if b.direction == 2 && r(6) < 0.5 {
                    d = [-d[0], -d[1], -d[2]];
                }
                // (a spot light's own direction doesn't turn it, ref LE2)
                rotate(d, b.rot)
            }
            3 => {
                // in the emitter's xy plane, tilted out of it by the spread
                let ang = std::f32::consts::TAU * r(5);
                let tilt = (r(4) - 0.5) * b.spread * std::f32::consts::PI;
                rotate(
                    [ang.cos() * tilt.cos(), ang.sin() * tilt.cos(), tilt.sin()],
                    b.rot,
                )
            }
            4 if len3(off) > 1e-6 => {
                let l = len3(off);
                [off[0] / l, off[1] / l, off[2] / l]
            }
            _ => unit_sphere(r(4), r(5)),
        };
        // triangular over ±Velocity Random (ref quantiles, the reference's Velocity Distribution 0.5)
        let mut speed = b.velocity * (1.0 + b.velocity_random * (r(7) + r(13) - 1.0));
        // Layer RGB Usage (ref LG3-LG9, a grid over stripes of known colour): lightness is the
        // plain mean of R, G, B (pure red, green or blue all 1/3); Size × 2L, Velocity ×
        // (4L - 2) (backwards under 50%), rotation layer_turn(L); RGB - Size Vel Rot the same
        // from R, G and B; RGB - XYZ Velocity: Velocity / 2 × (2c - 1) along x, y, z, turned by
        // the emitter's rotation like Directional (Direction is ignored)
        let (mut size_k, mut turn, mut xyz) = (1.0, 0.0, None);
        if let Some(px) = pixel {
            let c = [px[0], px[1], px[2]].map(|v| v / px[3].max(1e-6));
            let l = (c[0] + c[1] + c[2]) / 3.0;
            match b.layer_usage {
                2 => size_k = 2.0 * l,
                3 => speed *= 4.0 * l - 2.0,
                4 => turn = layer_turn(l),
                5 | 6 => {
                    size_k = 2.0 * c[0];
                    speed *= 4.0 * c[1] - 2.0;
                    turn = layer_turn(c[2]);
                }
                7 | 8 => xyz = Some(rotate(c.map(|v| 0.5 * speed * (2.0 * v - 1.0)), b.rot)),
                _ => {}
            }
        }
        // ±Life Random, half uniform and half triangular (reference: 89.4% of 3 s ±50% still alive at 1.98 s)
        let lr = if r(14) < 0.5 {
            2.0 * r(8) - 1.0
        } else {
            r(8) + r(15) - 1.0
        };
        let m = b.velocity_from_motion;
        let d = xyz.unwrap_or([dir[0] * speed, dir[1] * speed, dir[2] * speed]);
        let v = [d[0] + evel[0] * m, d[1] + evel[1] * m, d[2] + evel[2] * m];
        // Color Random: the colour blended towards a random one by that much, each channel on its
        // own (ref CR1-CR6: a channel lands uniformly in c (1 - R) .. c (1 - R) + R; 100% is any
        // colour)
        let cr = b.color_random.clamp(0.0, 1.0);
        let rand = [r(12), r(20), r(21)];
        let (mut color, mut opacity) = (
            std::array::from_fn(|i| b.color[i].max(0.0) * (1.0 - cr) + cr * rand[i]),
            b.opacity * (1.0 - b.opacity_random * r(10)),
        );
        if let Some(px) = pixel {
            opacity *= px[3];
            if b.layer_color {
                color =
                    std::array::from_fn(|i| (px[i] / px[3]).max(0.0) * (1.0 - cr) + cr * rand[i]);
            }
        }
        let mut q = Particle {
            id,
            p: p0,
            v,
            age: 0.0,
            life: b.life * (1.0 + b.life_random * lr),
            size: b.size * (1.0 + b.size_random * (2.0 * r(9) - 1.0)).max(0.0) * size_k,
            opacity,
            color,
            grad: r(11),
            rot: b.rot_z + b.rot_random * (2.0 * r(16) - 1.0) + turn,
            spin: b.spin * (1.0 + b.spin_random * (2.0 * r(17) - 1.0)),
        };
        // Its part of the step under the same forces as everyone (air resistance, wind, turbulence,
        // gravity), so a stream stays continuous: moved in a straight line at its launch speed
        // instead (the reference's Linear), particles born either side of a frame part and the
        // stream breaks into dashes (90 px gaps at 1300 px/s, Air Resistance 0.5).
        // ponytail: no floor in that part of a step; a bounce waits for the next.
        let t1 = (st.k + 1) as f32 * dt;
        advance(&mut q, b, b.turbulence, b.air, b.gravity * GRAVITY, t1, age);
        st.ps.push(q);
        if b.emitter_type == 3 {
            st.layer_uv.push(uv);
        }
    }
}

/// Improved-Perlin gradients (cube edges, four repeated to make 16).
const GRAD: [[f32; 3]; 16] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [0.0, -1.0, 1.0],
    [0.0, -1.0, -1.0],
];
/// Scales the noise to the spread of the random-unit-gradient noise the turbulence was tuned with.
const NOISE_GAIN: f32 = 1.135;

/// Three independent smooth 3D gradient noises (one per axis) from one lattice walk: each
/// corner's hash picks a gradient per axis. In about [-1, 1].
fn noise3v(p: [f32; 3]) -> [f32; 3] {
    let fl = p.map(f32::floor);
    let f = [p[0] - fl[0], p[1] - fl[1], p[2] - fl[2]];
    let i = fl.map(|x| x as i32);
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (fade(f[0]), fade(f[1]), fade(f[2]));
    let mut acc = [[0.0f32; 3]; 8];
    for (n, a) in acc.iter_mut().enumerate() {
        let (cx, cy, cz) = ((n & 1) as i32, ((n >> 1) & 1) as i32, ((n >> 2) & 1) as i32);
        let h = mix(((i[0] + cx) as u32 as u64)
            | (((i[1] + cy) as u32 as u64) << 21) ^ (((i[2] + cz) as u32 as u64) << 42));
        let d = [f[0] - cx as f32, f[1] - cy as f32, f[2] - cz as f32];
        for (ax, o) in a.iter_mut().enumerate() {
            let g = GRAD[((h >> (4 * ax)) & 15) as usize];
            *o = g[0] * d[0] + g[1] * d[1] + g[2] * d[2];
        }
    }
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let mut out = [0.0f32; 3];
    for (ax, o) in out.iter_mut().enumerate() {
        let c = |n: usize| acc[n][ax];
        let x0 = l(l(c(0), c(1), u), l(c(2), c(3), u), v);
        let x1 = l(l(c(4), c(5), u), l(c(6), c(7), u), v);
        *o = l(x0, x1, w) * NOISE_GAIN;
    }
    out
}

/// The turbulence wind at `p` and time `t`: fractal noise, one channel per axis.
fn turbulence(b: &Params, p: [f32; 3], t: f32, amount: f32) -> [f32; 3] {
    let cell = TURB_CELL / b.turb_scale.max(0.1);
    let evo = (t * b.turb_evolution + b.turb_offset) / 100.0;
    // Offset X / Y / Z move the field (px; ref TB3: Offset X 100 moves it 90-135 px right)
    let q = [0, 1, 2].map(|i| (p[i] - b.turb_shift[i]) / cell);
    let mut out = [0.0f32; 3];
    let (mut amp, mut freq, mut total) = (1.0f32, 1.0f32, 0.0f32);
    for _ in 0..b.turb_complexity {
        let n = noise3v([q[0] * freq, q[1] * freq, q[2] * freq + evo]);
        for ax in 0..3 {
            out[ax] += amp * n[ax];
        }
        total += amp;
        amp *= b.turb_octave_mult;
        freq *= b.turb_octave_scale;
    }
    let k = amount * TURB_SPEED / total.max(1e-6);
    out.map(|x| x * k)
}

/// One step of a particle's motion: wind (plus turbulence of strength `turb`) through quadratic
/// air resistance `air`, then gravity `g` (px/s²).
#[inline(always)]
fn advance(q: &mut Particle, b: &Params, turb: f32, air: f32, g: f32, t1: f32, dt: f32) {
    let mut w = b.wind;
    if turb != 0.0 {
        let tw = turbulence(b, q.p, t1, turb);
        w = [w[0] + tw[0], w[1] + tw[1], w[2] + tw[2]];
    }
    let rel = [q.v[0] - w[0], q.v[1] - w[1], q.v[2] - w[2]];
    // exact decay of the relative speed over the step at its current drag rate (stable for big k)
    let keep = (-air * (DRAG_LIN + DRAG_QUAD * len3(rel)) * dt).exp();
    for i in 0..3 {
        q.v[i] = w[i] + rel[i] * keep;
    }
    q.v[1] += g * dt;
    for i in 0..3 {
        q.p[i] += q.v[i] * dt;
    }
    // gravity integrated exactly over the step (reference: 100 px after 1 s at Gravity 100)
    q.p[1] -= 0.5 * g * dt * dt;
    q.age += dt;
}

/// The particles `t` seconds after `st` (0 < t < a frame): each moved on along its velocity and
/// aged, for a time between frames (a layer in a faster comp, time-stretched or time-remapped).
/// Motion blur's sub-frame samples move them the same way; nothing is born or dies in between.
pub fn between(st: &State, t: f32) -> State {
    let mut s = st.clone();
    for q in s.ps.iter_mut().chain(s.aux.iter_mut()) {
        for i in 0..3 {
            q.p[i] += q.v[i] * t;
        }
        q.age += t;
    }
    s
}

/// A main particle's colour now (as drawn).
fn main_rgb(q: &Particle, b: &Params) -> [f32; 3] {
    if b.emitter_type == 3 && b.layer_color {
        return q.color;
    }
    let t = match b.set_color {
        1 => q.age / q.life.max(1e-6),
        2 => q.grad,
        _ => return q.color,
    };
    match &b.gradient {
        Some(g) => g.color(t),
        None => lerp3(b.color, b.end_color, t),
    }
}

/// The Color over Life gradient's opacity for `q` (1 without one, or with Set Color At Start).
fn grad_alpha(q: &Particle, b: &Params) -> f32 {
    match (&b.gradient, b.set_color) {
        (Some(g), 1) => g.alpha(q.age / q.life.max(1e-6)),
        (Some(g), 2) => g.alpha(q.grad),
        _ => 1.0,
    }
}

/// A Color over Life gradient as EffectCraft sends it: colour stops (position 0-1, straight RGB)
/// and opacity stops, each kept in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Grad {
    colors: Vec<(f32, [f32; 3])>,
    opacities: Vec<(f32, f32)>,
}

impl Grad {
    /// From `{"colors": [[pos, [r, g, b, a]]...], "opacities": [[pos, a]...]}`; None when it has
    /// no colour stops (then Color to Gradient End Color stands in).
    pub fn from_json(text: &str) -> Option<Grad> {
        let j: Value = serde_json::from_str(text).ok()?;
        let num = |v: &Value| v.as_f64().filter(|x| x.is_finite()).map(|x| x as f32);
        let mut colors: Vec<(f32, [f32; 3])> = j["colors"]
            .as_array()?
            .iter()
            .take(256)
            .filter_map(|s| {
                let c = &s[1];
                Some((num(&s[0])?, [num(&c[0])?, num(&c[1])?, num(&c[2])?]))
            })
            .collect();
        let mut opacities: Vec<(f32, f32)> = j["opacities"]
            .as_array()
            .map(|a| {
                a.iter()
                    .take(256)
                    .filter_map(|s| Some((num(&s[0])?, num(&s[1])?)))
                    .collect()
            })
            .unwrap_or_default();
        if colors.is_empty() {
            return None;
        }
        colors.sort_by(|a, b| a.0.total_cmp(&b.0));
        opacities.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(Grad { colors, opacities })
    }

    pub fn color(&self, t: f32) -> [f32; 3] {
        stop_at(&self.colors, t, lerp3).unwrap_or([1.0; 3])
    }

    pub fn alpha(&self, t: f32) -> f32 {
        stop_at(&self.opacities, t, |a, b, f| a + (b - a) * f).unwrap_or(1.0)
    }
}

/// The value at `t` of stops in order: the ends held, linear between.
fn stop_at<T: Copy>(stops: &[(f32, T)], t: f32, lerp: impl Fn(T, T, f32) -> T) -> Option<T> {
    let i = stops.partition_point(|s| s.0 < t);
    match (i.checked_sub(1).and_then(|j| stops.get(j)), stops.get(i)) {
        (Some(a), Some(b)) => Some(lerp(a.1, b.1, (t - a.0) / (b.0 - a.0).max(1e-9))),
        (a, b) => a.or(b).map(|s| s.1),
    }
}

/// The ids of the parameters sent in `texts`, in order (text fields, and gradients when the host
/// takes them).
pub fn text_ids(gradients: bool) -> Vec<&'static str> {
    DEFS.iter()
        .filter(|p| matches!(p.kind, Text(_)) || (gradients && matches!(p.kind, Gradient(_))))
        .map(|p| p.id)
        .collect()
}

/// The aux particles main particle `q` emitted during the step that took it from `p0` to its
/// position now (age `q.age - dt` to `q.age`). Emission is at Particles/sec within the Start/Stop
/// window of its life, spaced evenly in time from a per-particle phase, so a trail is continuous
/// whatever the frame rate; each starts on the path where the main particle was at its birth.
fn aux_births<'a>(
    q: &'a Particle,
    p0: [f32; 3],
    b: &'a Params,
    dt: f32,
) -> impl Iterator<Item = Particle> + 'a {
    let (age1, age0) = (q.age, q.age - dt);
    let (w0, w1) = (b.aux_window[0] * q.life, b.aux_window[1] * q.life);
    let (lo, hi) = (age0.max(w0), age1.min(w1));
    let phase = rnd(b.seed, q.id, 40);
    let (n0, n1) = if hi > lo {
        (
            ((lo - w0) * b.aux_rate + phase).floor() as u64,
            ((hi - w0) * b.aux_rate + phase).floor() as u64,
        )
    } else {
        (0, 0)
    };
    let from = main_rgb(q, b);
    (n0..n1).map(move |k| {
        let id = (q.id << 24) ^ k ^ 0x5bd1_e995_0000_0000;
        let s = b.seed ^ 0xa0c1;
        let r = |ch| rnd(s, id, ch);
        let born = w0 + (k as f32 + 1.0 - phase) / b.aux_rate;
        let f = ((born - age0) / dt).clamp(0.0, 1.0);
        let age = (age1 - born).max(0.0);
        let d = unit_sphere(r(1), r(2));
        let m = b.aux_inherit;
        let v = [
            d[0] * b.aux_velocity + q.v[0] * m,
            d[1] * b.aux_velocity + q.v[1] * m,
            d[2] * b.aux_velocity + q.v[2] * m,
        ];
        let at = lerp3(p0, q.p, f);
        let lr = if r(3) < 0.5 {
            2.0 * r(4) - 1.0
        } else {
            r(4) + r(5) - 1.0
        };
        Particle {
            id,
            p: [at[0] + v[0] * age, at[1] + v[1] * age, at[2] + v[2] * age],
            v,
            age,
            life: b.aux_life * (1.0 + b.aux_life_random * lr),
            size: b.aux_size * (1.0 + b.aux_size_random * (2.0 * r(6) - 1.0)).max(0.0),
            opacity: b.aux_opacity * (1.0 - b.aux_opacity_random * r(7)),
            color: lerp3(b.aux_color, from, b.aux_from_main),
            grad: r(8),
            rot: 0.0,
            spin: 0.0,
        }
    })
}

/// Advances `st` from frame k (params `a`) to frame k + 1 (params `b`), `dt` seconds.
pub fn step(st: &mut State, a: &Params, b: &Params, dt: f32) {
    step_with(st, a, b, dt, None);
}

/// [`step`] with the emitter layer at frame k + 1 (used when Emitter Type is Layer).
pub fn step_with(st: &mut State, a: &Params, b: &Params, dt: f32, layer: Option<&EmitLayer>) {
    let t1 = (st.k + 1) as f32 * dt;
    if !st.aux.is_empty() {
        let g = b.aux_gravity * GRAVITY;
        st.aux
            .par_iter_mut()
            .for_each(|q| advance(q, b, b.aux_turbulence, b.aux_air, g, t1, dt));
        st.aux.retain(|q| q.age <= q.life + 1e-4);
    }
    if b.aux_on {
        st.prev.clear();
        st.prev.extend(st.ps.iter().map(|q| q.p));
    }
    let g = b.gravity * GRAVITY;
    let k = st.k;
    if b.floors.iter().any(|f| f.placed) {
        st.contact.resize(st.ps.len(), (0, 0));
        st.ps
            .par_iter_mut()
            .zip(st.contact.par_iter_mut())
            .for_each(|(q, c)| floor_step(q, c, b, &b.floors, g, t1, dt, k));
    } else {
        st.contact.clear();
        st.ps
            .par_iter_mut()
            .for_each(|q| advance(q, b, b.turbulence, b.air, g, t1, dt));
    }
    if b.aux_on {
        let born: Vec<Particle> = st
            .ps
            .par_iter()
            .zip(st.prev.par_iter())
            .flat_map_iter(|(q, &p0)| aux_births(q, p0, b, dt))
            .collect();
        st.aux.extend(born);
    }
    let alive = |q: &Particle| q.age <= q.life + 1e-4;
    if !st.contact.is_empty() {
        let (ps, mut i) = (&st.ps, 0);
        st.contact.retain(|_| {
            i += 1;
            alive(&ps[i - 1])
        });
    }
    if b.emitter_type != 3 {
        st.layer_uv.clear();
    } else if st.layer_uv.len() == st.ps.len() {
        let (ps, mut i) = (&st.ps, 0);
        st.layer_uv.retain(|_| {
            i += 1;
            alive(&ps[i - 1])
        });
    } else {
        st.layer_uv.clear();
    }
    st.ps.retain(alive);
    // Layer: Particles/sec over the whole layer, so its opaque share of it emits (ref / here,
    // whole-frame alpha over seven references, L1–L7: 0.84–1.10, mean 1.01)
    let grid = b
        .grid
        .filter(|_| matches!(b.emitter_type, 1 | 2) || (b.emitter_type == 3 && layer.is_some()));
    let share = match layer {
        // Light(s): Particles/sec from each light, times its Intensity (ref LE1: a 50% spot emits
        // half as many as a 100% point light)
        _ if b.emitter_type == 4 => b.lights.iter().map(|l| l.intensity.max(0.0)).sum(),
        // Text/Mask Edges: Particles/sec from the outline, however long
        _ if b.emitter_type != 3 || grid.is_some() || b.edges => 1.0,
        Some(l) if !l.opaque.is_empty() => l.opaque.len() as f32 / (l.w * l.h) as f32,
        _ => 0.0,
    };
    let m = 0.5 * ((a.rate + b.rate) * share) as f64 * dt as f64;
    let n = if b.explode {
        if st.k == 0 {
            (b.rate * share).round() as usize
        } else {
            0
        }
    } else {
        st.acc += m;
        let n = st.acc.floor();
        st.acc -= n;
        n as usize
    };
    // On a Grid (ref LG11-LG15): each emission event (Particles/sec of them) is, by Sequencing
    // Type, the whole grid when a cycle starts (Periodic Burst: 96/s over 32 cells bursts 3 times
    // a second, the first 1/96 s in), the next cell in x, y, z order (Traverse), a random cell, or
    // the next x-y cell in every Z plane (Z Sync Traverse). Explode: the grid once, all born at 0.
    let mut births = Vec::with_capacity(n);
    match grid {
        Some(([nx, ny, nz], seq)) => {
            let cells = nx * ny * nz;
            if b.explode {
                if st.k == 0 && b.rate > 0.0 {
                    births.extend((0..cells).map(|c| (1.0, c)));
                }
            } else {
                // oldest first, each at its exact time: the accumulator passed its whole number
                // (st.acc + i) / m of the step before its end
                for i in (0..n).rev() {
                    let frac = ((st.acc + i as f64) / m.max(1e-12)).min(1.0) as f32;
                    let e = st.events;
                    st.events += 1;
                    match seq {
                        0 if e.is_multiple_of(cells as u64) => {
                            births.extend((0..cells).map(|c| (frac, c)))
                        }
                        0 => {}
                        1 => births.push((frac, (e % cells as u64) as u32)),
                        2 => {
                            let c = (rnd(b.seed ^ 0x6e1d, e, 0) * cells as f32) as u32;
                            births.push((frac, c.min(cells - 1)));
                        }
                        _ => {
                            let xy = (e % (nx * ny) as u64) as u32;
                            births.extend((0..nz).map(|k| (frac, xy + k * nx * ny)));
                        }
                    }
                }
            }
        }
        None => births.extend((0..n).map(|i| (i as f32 / n as f32, u32::MAX))),
    }
    emit(st, a, b, dt, &births, layer);
    st.k += 1;
}

// ---------------------------------------------------------------------------------------------
// Rendering

fn smoothstep(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// A projected particle in buffer pixels.
#[derive(Clone, Copy, Default)]
struct Dot {
    x: f32,
    y: f32,
    /// outer radius of the sphere and of what's drawn
    r: f32,
    reach: f32,
    /// feather width of the sphere falloff
    w: f32,
    rgb: [f32; 3],
    a: f32,
    depth: f32,
    /// motion blur: offset of each further sub-frame sample (buffer px) and the sample count
    step: [f32; 2],
    n: u32,
    /// big particles: index of their falloff table in `Scratch::luts` (u32::MAX: none);
    /// sprites: index of their frame in `Sprites`
    lut: u32,
    /// drawn at all (off screen, behind the camera or transparent: no)
    keep: bool,
    /// cos and sin of the rotation, and its change per motion blur sample (sprites)
    rot: [f32; 2],
    spin: f32,
    /// sprites out of focus: the Gaussian blur's sigma (buffer px)
    blur: f32,
    /// a self-shadow: darkens what is drawn before it (rgb: its colour, a: its opacity, w: the
    /// soft edge's width)
    shadow: bool,
}

/// Buffers `render` reuses from frame to frame: allocating them fresh each time cost more than
/// drawing (Windows page-faults every new multi-megabyte allocation).
#[derive(Default)]
pub struct Scratch {
    dots: Vec<Dot>,
    /// blurred sprite levels this render draws (see `sprite_blur_key`), sorted by key
    sblur: Vec<(SpriteBlurKey, Level)>,
    order: Vec<u64>,
    luts: Vec<f32>,
    big: Vec<u32>,
    bands: Vec<Vec<u32>>,
}

/// One frame of the sprite layer: its visible part (at `x0`, `y0` in the image EffectCraft
/// handed over), with box-filtered half-size levels down to a pixel, so a small particle samples
/// a level about its own size. No levels: the layer was empty at that frame.
struct Sprite {
    x0: f32,
    y0: f32,
    levels: Vec<Level>,
}

/// The sprite layer's frames in use, by frame number, and the box around all their visible
/// parts. EffectCraft doesn't say how big the layer itself is, so where the reference fits the sprite
/// layer's width to 2 × Size, this fits the box's width (the same when the sprite fills its
/// layer), and centres the box on the particle, so a sprite's frames keep their size and place.
#[derive(Default)]
pub struct Sprites {
    /// sorted by frame
    frames: Vec<(i64, Sprite)>,
    /// x0, y0, x1, y1 of the visible parts (x0 > x1: none)
    bx: [f32; 4],
}

impl Sprites {
    pub fn clear(&mut self) {
        self.frames.clear();
        self.bx = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn has(&self, f: i64) -> bool {
        self.frames.binary_search_by_key(&f, |e| e.0).is_ok()
    }

    /// The frame numbers held, in order.
    pub fn frames(&self) -> impl Iterator<Item = i64> + '_ {
        self.frames.iter().map(|e| e.0)
    }

    /// The held frame nearest `f` (the earlier one on a tie); None when none is held.
    fn nearest(&self, f: i64) -> Option<usize> {
        match self.frames.binary_search_by_key(&f, |e| e.0) {
            Ok(i) => Some(i),
            Err(0) => (!self.frames.is_empty()).then_some(0),
            Err(i) if i == self.frames.len() => Some(i - 1),
            Err(i) => Some(if f - self.frames[i - 1].0 <= self.frames[i].0 - f {
                i - 1
            } else {
                i
            }),
        }
    }

    /// Adds (or replaces) frame `f` from the sprite layer image `img` (`w` × `h`, as EffectCraft
    /// hands it over: the layer in this frame's grid).
    pub fn insert(&mut self, f: i64, img: &[Px], w: usize, h: usize) {
        if self.frames.is_empty() {
            self.clear();
        }
        // the visible part's box, rows in parallel
        let (x0, y0, x1, y1) = img
            .par_chunks(w.max(1))
            .take(h)
            .enumerate()
            .filter_map(|(y, row)| {
                let a = row.iter().position(|p| p[3] > 1.0 / 512.0)?;
                let b = row.iter().rposition(|p| p[3] > 1.0 / 512.0)?;
                Some((a, y, b + 1, y + 1))
            })
            .reduce(
                || (usize::MAX, usize::MAX, 0, 0),
                |p, q| (p.0.min(q.0), p.1.min(q.1), p.2.max(q.2), p.3.max(q.3)),
            );
        let mut levels = Vec::new();
        if x0 < x1 {
            let (sw, sh) = (x1 - x0, y1 - y0);
            let mut px = Vec::with_capacity(sw * sh);
            for y in y0..y1 {
                px.extend_from_slice(&img[y * w + x0..y * w + x1]);
            }
            levels.push(Level::new(sw, sh, &px));
            while levels.last().is_some_and(|l| l.w > 1 || l.h > 1) {
                let l = levels.last().unwrap();
                let (nw, nh) = (l.w.div_ceil(2), l.h.div_ceil(2));
                let mut px = Vec::with_capacity(nw * nh);
                for y in 0..nh {
                    for x in 0..nw {
                        let mut o = [0.0f32; 4];
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let p = l.at((2 * x + dx).min(l.w - 1), (2 * y + dy).min(l.h - 1));
                            for i in 0..4 {
                                o[i] += 0.25 * p[i];
                            }
                        }
                        px.push(o);
                    }
                }
                levels.push(Level::new(nw, nh, &px));
            }
            let b = &mut self.bx;
            *b = [
                b[0].min(x0 as f32),
                b[1].min(y0 as f32),
                b[2].max(x1 as f32),
                b[3].max(y1 as f32),
            ];
        }
        let sp = Sprite {
            x0: x0 as f32,
            y0: y0 as f32,
            levels,
        };
        match self.frames.binary_search_by_key(&f, |e| e.0) {
            Ok(i) => self.frames[i].1 = sp,
            Err(i) => self.frames.insert(i, (f, sp)),
        }
    }

    /// Any frame with something visible.
    fn visible(&self) -> bool {
        self.bx[0] < self.bx[2]
    }
}

/// The sprite clip time (s) particle `q` shows at comp time `t` (Time Sampling, measured on reference renders
/// with a 2 s clip whose colour encodes its time, refs TS1–TS11); None once a Play Once clip has
/// played out. Random and Split Clip pick per particle; Birth Time holds the frame at its birth.
fn sprite_time(q: &Particle, b: &Params, t: f32) -> Option<f32> {
    let l = b.clip_len.max(1e-3);
    let n = b.clip_count;
    let r = |ch| rnd(b.sprite_seed ^ 0x5b17_e5ee, q.id, ch);
    let seg = l / n;
    let clip = (r(1) * n).floor().min(n - 1.0) * seg;
    let life = q.life.max(1e-6);
    let once = |s: f32, end: f32| (s < end).then_some(s);
    // the age a hair over, so a particle a whole number of clips old (summed frame steps:
    // 1.99999 s) shows the clip's first frame like the reference's, not past its end
    let age = q.age + 1e-4;
    match b.time_sampling {
        1 => once(age, l),
        2 => Some(age.rem_euclid(l)),
        3 => Some(age / life * l),
        4 => Some(r(0) * l),
        // a whole clip's length from a random point, wrapping (reference: every particle under 2 s
        // old shows, 2 s itself too, at (r + age) mod 2)
        5 => once(age, l + 2e-4).map(|a| (r(0) * l + a).rem_euclid(l)),
        6 => Some((r(0) * l + age).rem_euclid(l)),
        7 => once(age, seg).map(|a| clip + a),
        8 => Some(clip + age.rem_euclid(seg)),
        9 => Some(clip + age / life * seg),
        10 => Some(t - q.age),
        _ => Some(t),
    }
}

/// The frame of a clip time `s` at `dt` seconds a frame.
fn clip_frame(s: f32, dt: f32) -> i64 {
    (s / dt + 1e-3).floor() as i64
}

/// The sprite frames the particles of `st` show (sorted, each once), for Time Sampling modes
/// other than Current Time.
pub fn sprite_frames(st: &State, b: &Params, dt: f32, out: &mut Vec<i64>) {
    let t = st.k as f32 * dt;
    out.clear();
    out.extend(
        st.ps
            .iter()
            .filter_map(|q| sprite_time(q, b, t).map(|s| clip_frame(s, dt))),
    );
    out.sort_unstable();
    out.dedup();
}

struct Level {
    w: usize,
    h: usize,
    /// row stride of `pad`: w + 2 (one transparent pixel on each side)
    pw: usize,
    pad: Vec<Px>,
}

impl Level {
    fn new(w: usize, h: usize, px: &[Px]) -> Level {
        let pw = w + 2;
        let mut pad = vec![[0.0f32; 4]; pw * (h + 2)];
        for y in 0..h {
            pad[(y + 1) * pw + 1..(y + 1) * pw + 1 + w].copy_from_slice(&px[y * w..(y + 1) * w]);
        }
        Level { w, h, pw, pad }
    }

    #[inline(always)]
    fn at(&self, x: usize, y: usize) -> Px {
        self.pad[(y + 1) * self.pw + x + 1]
    }

    /// Bilinear sample at (x, y) in this level's pixels (centres at +0.5); transparent outside.
    #[inline(always)]
    fn sample(&self, x: f32, y: f32) -> Px {
        // in the padded grid, clamped into its transparent border: no bounds branches
        let fx = (x + 0.5).clamp(0.0, self.w as f32 + 0.999);
        let fy = (y + 0.5).clamp(0.0, self.h as f32 + 0.999);
        // both are clamped into 0..=w / 0..=h, so the truncation can't overflow
        let (x0, y0) = (fx as i32 as usize, fy as i32 as usize);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let i = y0 * self.pw + x0;
        let (top, bot) = (&self.pad[i..i + 2], &self.pad[i + self.pw..i + self.pw + 2]);
        let (a, b, c, d) = (top[0], top[1], bot[0], bot[1]);
        // weights of the four texels
        let (wa, wb, wc, wd) = (
            (1.0 - tx) * (1.0 - ty),
            tx * (1.0 - ty),
            (1.0 - tx) * ty,
            tx * ty,
        );
        std::array::from_fn(|k| a[k] * wa + b[k] * wb + c[k] * wc + d[k] * wd)
    }
}

/// Sphere: flat core, then a smoothstep to zero over `w` ending half a pixel outside R
/// (fitted on the reference's 10–100 px profiles at feather 0, 50 and 100 and the coverage of 1–20 px
/// spheres: w = min(f·R + 3.25, R + 0.5); 0.17.2 refit on the reference's sharp 3-15 px spheres (DOF0):
/// within 0.4% of their alpha, was 3.75 and 4-8.5% light).
fn sphere(d: f32, r: f32, w: f32) -> f32 {
    1.0 - smoothstep((d - (r + 0.5 - w)) / w)
}

/// The reference draws spheres under 2 px at about half their shape's coverage (1 px: 0.50, 1.5 px: 0.57).
// ponytail: a ramp through two measurements; refit if tiny particles look off.
fn tiny(r: f32) -> f32 {
    (0.5 + (r - 1.5)).clamp(0.5, 1.0)
}

/// Per-particle falloff table over squared distance, for particles big enough that evaluating
/// the shape once per entry beats once per pixel.
const LUT: usize = 256;
const LUT_REACH: f32 = 12.0;

/// A particle's coverage at `dist`: its shape, and for Glow Sphere the glow, which is drawn
/// after the sphere with its own blend mode (reference: a glow falls off as a smoothstep
/// over Glow Size × R × Glow Feather, at least 3 px, ending half a pixel outside Glow Size × R,
/// at Glow Opacity; 13 references within 0.006 mean).
fn shape(d: &Dot, b: &Params, dist: f32) -> [f32; 2] {
    match b.ptype {
        1 => {
            let gr = d.r * b.glow_size;
            let w = (gr * b.glow_feather).max(3.0);
            let glow = b.glow_opacity * (1.0 - smoothstep((dist - (gr + 0.5 - w)) / w));
            let s = sphere(dist, d.r, d.w);
            if glow_split(b) {
                [s, glow]
            } else {
                // both Normal: one composite of the combined coverage is the same
                [s + glow - s * glow, 0.0]
            }
        }
        // Star: its glow only (the core and rays are drawn per pixel in `star_cov`)
        4 => {
            let gr = 1.2 * d.r * b.glow_size;
            let w = (gr * b.glow_feather).max(3.0);
            [
                b.glow_opacity * (1.0 - smoothstep((dist - (gr + 0.5 - w)) / w)),
                0.0,
            ]
        }
        _ => [sphere(dist, d.r, d.w), 0.0],
    }
}

/// A Streaks particle's pattern, the same for every particle: per dot its offset in particle radii
/// and its size as a share of the centre dot's. Measured on reference renders (refs SK1–SK9):
/// one dot at the centre, 0.615 × Size × Streak Size in radius with the sphere's feather (Streak
/// Size 30/60/100); the others spread like a ball of radius Size seen from the front (median
/// 0.56–0.6 of Size out for 30 dots, two seeds) and 0.2–0.75 the centre dot's size, evenly
/// (total alpha of 30 dots: The reference 1414–1561, here 1453–1616 over four seeds); moving with motion
/// blur they smear into parallel streaks. The pattern itself is the reference's own random, so
/// only these statistics are matched.
fn streak_pattern(b: &Params) -> Vec<[f32; 3]> {
    (0..b.streak_count as u64)
        .map(|k| {
            if k == 0 {
                return [0.0, 0.0, 1.0];
            }
            let r = |ch| rnd(b.streak_seed ^ 0x57ea_c1e7, k, ch);
            let (d, rad) = (unit_sphere(r(1), r(2)), r(3).cbrt());
            let u = r(4);
            [d[0] * rad, d[1] * rad, 0.2 + 0.55 * u]
        })
        .collect()
}

/// Glow spheres whose sphere and glow need separate composites (not both Normal).
fn glow_split(b: &Params) -> bool {
    b.ptype == 1 && (b.blend != 0 || b.glow_blend != 0)
}

/// A Star's coverage at offset (dx, dy) from its centre: [core and rays, glow], or one
/// combined coverage when both blend modes are Normal. Measured on reference renders (refs
/// ST1–ST9, R = Size): a disk ending at 0.6·R + 0.5 with an edge 3.1 + 0.12·R wide (max |Δ|
/// 0.004 at R = 20, 50, 100); four rays along the particle's rotated axes (Rotate Z turns them
/// clockwise), each a wedge 0.3·R wide at the centre narrowing to nothing at 3·R whatever the
/// glow; the glow of a Glow Sphere reaching 1.2 × Glow Size × R.
#[inline(always)]
fn star_cov(d: &Dot, glow: f32, d2: f32, lx: f32, ly: f32, merge: bool) -> [f32; 2] {
    let rc = 0.6 * d.r;
    let mut c = if d2 < (rc + 0.5) * (rc + 0.5) {
        sphere(d2.sqrt(), rc, d.w)
    } else {
        0.0
    };
    // the rays: only near the particle's own axes (lx, ly: the offset in the star's frame)
    let (lx, ly) = (lx.abs(), ly.abs());
    if lx.min(ly) < 0.15 * d.r + 0.5 {
        let len = 3.0 * d.r;
        let ray = |along: f32, across: f32| {
            if along >= len {
                return 0.0;
            }
            let h = 0.15 * d.r * (1.0 - along / len);
            (h.min(across + 0.5) - (-h).max(across - 0.5)).clamp(0.0, 1.0)
        };
        c = c.max(ray(lx, ly)).max(ray(ly, lx));
    }
    if merge {
        [c + glow - c * glow, 0.0]
    } else {
        [c, glow]
    }
}

/// One Star over rows `y0..y1` of the band, motion blur included (the average of its samples).
fn star_rows<const M: usize>(
    c: &Ctx,
    d: &Dot,
    glow_table: Option<&[f32; LUT + 1]>,
    rows: &mut [Px],
    y_base: usize,
    y0: usize,
    y1: usize,
) {
    let w = c.w;
    let reach2 = d.reach * d.reach;
    let k_lut = LUT as f32 / reach2;
    let merge = c.b.blend == 0 && c.b.glow_blend == 0;
    let glow_on = c.b.glow_opacity > 0.0;
    let glow = |d2: f32| match glow_table {
        _ if !glow_on => 0.0,
        Some(t) => lut_at(t, d2 * k_lut),
        None => shape(d, c.b, d2.sqrt())[0],
    };
    let ex = d.step[0] * (d.n - 1) as f32;
    let inv_n = 1.0 / d.n as f32;
    for y in y0..y1 {
        let row = &mut rows[(y - y_base) * w..(y - y_base + 1) * w];
        let ry = y as f32 + 0.5 - d.y;
        // a still star: only the circle's span of this row
        let (lo, hi) = if d.n == 1 {
            let half = (reach2 - ry * ry).max(0.0).sqrt();
            (d.x - half, d.x + half)
        } else {
            (d.x.min(d.x + ex) - d.reach, d.x.max(d.x + ex) + d.reach)
        };
        let x0 = (lo.floor().max(0.0) as usize).min(w);
        let x1 = (hi.ceil().max(0.0) as usize).min(w);
        let (cs, sn) = (d.rot[0], d.rot[1]);
        if d.n == 1 {
            // the star-frame offset steps by (cs, −sn) per pixel
            let rx0 = x0 as f32 + 0.5 - d.x;
            let (mut lx, mut ly) = (rx0 * cs + ry * sn, -rx0 * sn + ry * cs);
            let mut rx = rx0;
            for o in &mut row[x0..x1] {
                let d2 = rx * rx + ry * ry;
                if d2 < reach2 {
                    let s = star_cov(d, glow(d2), d2, lx, ly, merge);
                    put::<M>(o, d.rgb, d.a, s, c.b.glow_blend);
                }
                rx += 1.0;
                lx += cs;
                ly -= sn;
            }
            continue;
        }
        for (x, o) in row.iter_mut().enumerate().take(x1).skip(x0) {
            let rx = x as f32 + 0.5 - d.x;
            let mut cov = [0.0; 2];
            for k in 0..d.n {
                let (dx, dy) = (rx - d.step[0] * k as f32, ry - d.step[1] * k as f32);
                let d2 = dx * dx + dy * dy;
                if d2 < reach2 {
                    let (lx, ly) = (dx * cs + dy * sn, -dx * sn + dy * cs);
                    let s = star_cov(d, glow(d2), d2, lx, ly, merge);
                    cov[0] += s[0];
                    cov[1] += s[1];
                }
            }
            put::<M>(o, d.rgb, d.a * inv_n, cov, c.b.glow_blend);
        }
    }
}

/// The comp's camera layer as the Particles layer sees it (EffectCraft's `comp_camera`): layer
/// pixels (x, y, z away from the viewer) -> homogeneous X, Y, W, the point showing at (X / W,
/// Y / W) and W its depth in front of the camera; a length there shows `zoom / W` times as long.
/// The reference draws through the comp camera exactly so (ref CAM1-CAM3: grid points within 0.03
/// px of a camera model, sizes Size x zoom / depth within 1%).
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub m: [[f64; 4]; 3],
    pub zoom: f64,
    /// its depth of field when on: focus distance (comp px from the eye), aperture (comp px),
    /// Blur Level (1 = 100%)
    pub dof: Option<[f64; 3]>,
    /// motion blur: its matrix at the shutter's start and end (a moving or turning camera
    /// smears what it sees; newer EffectCraft's comp_camera at a time)
    pub shutter: Option<[[[f64; 4]; 3]; 2]>,
}

impl Camera {
    /// How much bigger the reference draws a particle at depth `w` (comp px) out of focus, in comp
    /// px of Size (refs DOF1-DOF8, `stats.py DOF`: Sphere particles at depths 1067-5667,
    /// focus 1600-4000, aperture 200-400, Blur Level 25-150%, zoom 1800-2667; within 1%):
    /// CoC x (0.050 + 0.006 u), CoC = Aperture x Blur Level x zoom x |w - focus| / (focus x w)
    /// (the thin lens's circle of confusion), u = Blur Level x zoom / focus. It then draws it
    /// fully feathered, at min(1, (Size / growth)^2) of its opacity.
    fn defocus(&self, w: f64) -> f32 {
        match self.dof {
            Some([f, a, bl]) if f > 0.0 && w > 0.0 => {
                let u = bl * self.zoom / f;
                (a * u * (w - f).abs() / w * (0.050 + 0.006 * u)) as f32
            }
            _ => 0.0,
        }
    }
}

/// Draws the particles into `px` (the layer is replaced): the aux particles, then the main ones
/// over them (trails behind what leaves them). `dt` is the frame duration (the motion blur
/// shutter is in frames). `bounded`: only the rows the particles cover are cleared and drawn
/// (the host makes the rest transparent: EffectCraft's `frame_bounds`), else all of `px`.
/// Returns what was drawn, `[x0, y0, x1, y1]` in `px` (end exclusive; empty `[0; 4]` for nothing).
#[allow(clippy::too_many_arguments)]
pub fn render(
    px: &mut [Px],
    w: usize,
    h: usize,
    st: &State,
    b: &Params,
    scale: f32,
    origin: [f32; 2],
    layer: [f32; 2],
    dt: f32,
    sc: &mut Scratch,
    sprites: &Sprites,
    layer_now: Option<&[[f32; 3]]>,
    cam: Option<&Camera>,
    bounded: bool,
) -> [u32; 4] {
    let aux = !st.aux.is_empty();
    // what has been cleared and drawn so far (empty: x0 >= x1)
    let mut area = [w, h, 0, 0];
    if aux {
        let al = b.aux_look();
        render_pass(
            px,
            w,
            h,
            &st.aux,
            cloud_look(&al).as_ref().unwrap_or(&al),
            scale,
            origin,
            layer,
            dt,
            st.k as f32 * dt,
            sc,
            (al.ptype == 2).then(cloud_sprites),
            None,
            cam,
            !bounded,
            bounded.then_some(&mut area),
        );
    }
    let cl = cloud_look(b);
    let sprites = match &cl {
        Some(_) => Some(cloud_sprites()),
        None => (b.ptype == 3).then_some(sprites),
    };
    let b = cl.as_ref().unwrap_or(b);
    render_pass(
        px,
        w,
        h,
        &st.ps,
        b,
        scale,
        origin,
        layer,
        dt,
        st.k as f32 * dt,
        sc,
        sprites,
        // Layer Sampling Current Time (ref LS7, LS8): the layer's colour now at each particle's
        // birth pixel, wherever the particle has gone since
        layer_now.filter(|c| b.layer_now && b.layer_color && c.len() == st.ps.len()),
        cam,
        !aux && !bounded,
        bounded.then_some(&mut area),
    );
    if area[0] < area[2] && area[1] < area[3] {
        area.map(|v| v as u32)
    } else {
        [0; 4]
    }
}

/// The cloud texture's half width in sizes (ref CL3: a Size 40 puff is gone by 1.6 x Size).
const CLOUD_EXTENT: f32 = 1.9;

/// Puffs drawn as sprites of the built-in cloud texture: the particle's colour through it
/// (Fill), always its one frame.
fn cloud_look(b: &Params) -> Option<Params> {
    (b.ptype == 2).then(|| Params {
        ptype: 3,
        sprite_mode: 2,
        time_sampling: 0,
        cloud: true,
        ..b.clone()
    })
}

/// The cloud texture (built once): a lumpy puff, a core lobe and seven around it, each solid
/// inside with a smoothstep edge, merged, with a little fine grain. The reference's puff is a
/// lumpy puff like it (CL1-CL3); this texture is our own, its average over each circle fitted on
/// The reference's Size 40 puff (CL3: solid to 1.1 x Size, gone by 1.6 x). 256 px = 2 x CLOUD_EXTENT sizes.
fn cloud_sprites() -> &'static Sprites {
    static S: std::sync::OnceLock<Sprites> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        const N: usize = 256;
        const EDGE: f32 = 0.4;
        // lobes: centre and radius (in sizes)
        let mut lobes = vec![([0.0f32, 0.0f32], 1.2f32)];
        for k in 0..7 {
            let a = k as f32 / 7.0 * std::f32::consts::TAU + 0.6 * (rnd(11, k, 0) - 0.5);
            let d = 0.7 + 0.25 * rnd(11, k, 1);
            lobes.push(([a.cos() * d, a.sin() * d], 0.7 + 0.2 * rnd(11, k, 2)));
        }
        // fine grain: smooth value noise on a 10 x 10 lattice
        let grain = |x: f32, y: f32| {
            let (gx, gy) = ((x + 1.0) * 5.0, (y + 1.0) * 5.0);
            let (ix, iy) = (gx.floor(), gy.floor());
            let (fx, fy) = (smoothstep(gx - ix), smoothstep(gy - iy));
            let v = |i: f32, j: f32| rnd(12, (j * 16.0 + i) as u64, 0);
            let top = v(ix, iy) + (v(ix + 1.0, iy) - v(ix, iy)) * fx;
            let bot = v(ix, iy + 1.0) + (v(ix + 1.0, iy + 1.0) - v(ix, iy + 1.0)) * fx;
            top + (bot - top) * fy
        };
        let at = |i: usize| (i as f32 + 0.5) / N as f32 * 2.0 - 1.0;
        let mut img = vec![[0.0f32; 4]; N * N];
        for y in 0..N {
            for x in 0..N {
                let (px, py) = (at(x) * CLOUD_EXTENT, at(y) * CLOUD_EXTENT);
                let mut clear = 1.0;
                for (c, r) in &lobes {
                    let d = (px - c[0]).hypot(py - c[1]);
                    clear *= smoothstep((d - (r - EDGE)) / EDGE);
                }
                let a = ((1.0 - clear) * (0.88 + 0.12 * grain(at(x), at(y))) * 1.1).min(1.0);
                img[y * N + x] = [a; 4];
            }
        }
        // the corners just visible, so the sprite's box is the whole texture
        for i in [0, N - 1, N * (N - 1), N * N - 1] {
            img[i] = [0.0025; 4];
        }
        let mut sp = Sprites::default();
        sp.insert(0, &img, N, N);
        sp
    })
}

/// One population drawn over `px` (cleared first if `clear`).
#[allow(clippy::too_many_arguments)]
fn render_pass(
    px: &mut [Px],
    w: usize,
    h: usize,
    ps: &[Particle],
    b: &Params,
    scale: f32,
    origin: [f32; 2],
    layer: [f32; 2],
    dt: f32,
    time: f32,
    sc: &mut Scratch,
    sprites: Option<&Sprites>,
    recolor: Option<&[[f32; 3]]>,
    cam: Option<&Camera>,
    clear: bool,
    area: Option<&mut [usize; 4]>,
) {
    let zoom = layer[0] * 50.0 / 36.0;
    // a point's depth in front of the camera (sorting, size)
    let cam_w = |c: &Camera, p: [f32; 3]| {
        let w = c.m[2];
        w[0] * p[0] as f64 + w[1] * p[1] as f64 + w[2] * p[2] as f64 + w[3]
    };
    let (cx, cy) = (layer[0] * 0.5, layer[1] * 0.5);
    // Sprite particles without a visible sprite layer draw nothing.
    if b.ptype == 3 && !sprites.is_some_and(Sprites::visible) {
        if clear {
            px.par_iter_mut().for_each(|p| *p = [0.0; 4]);
        }
        return;
    }
    let sprite_diag = sprites.map_or(1.0, |s| {
        let (bw, bh) = (s.bx[2] - s.bx[0], s.bx[3] - s.bx[1]);
        bw.hypot(bh) / bw
    });
    // a dot's box, streak included: x0, x1, y0, y1
    let bbox = |d: &Dot| {
        let (ex, ey) = (
            d.x + d.step[0] * (d.n - 1) as f32,
            d.y + d.step[1] * (d.n - 1) as f32,
        );
        [
            d.x.min(ex) - d.reach,
            d.x.max(ex) + d.reach,
            d.y.min(ey) - d.reach,
            d.y.max(ey) + d.reach,
        ]
    };
    // Streaks: each particle is a fixed pattern of sphere dots (one dot for other types)
    let pattern = if b.ptype == 5 {
        streak_pattern(b)
    } else {
        vec![[0.0, 0.0, 1.0]]
    };
    let (m, pattern) = (pattern.len(), &pattern);
    // self-shadow: the light they fall away from (default: at the camera's eye)
    let shadows = b.self_shadow;
    let eye = match cam {
        Some(c) => camera_eye(&c.m),
        None => [cx, cy, -zoom],
    };
    let sh_light = b.shadow_light.unwrap_or(ShadeLight {
        kind: 2,
        pos: eye,
        dir: [0.0, 0.0, 1.0],
        color: [0.0; 3],
        half: 0.0,
        feather: 0.0,
    });
    let m_all = m + shadows.is_some() as usize;
    sc.dots.resize(ps.len() * m_all, Dot::default());
    sc.dots
        .par_chunks_mut(m_all)
        .zip(ps.par_iter().enumerate())
        .with_min_len((512 / m_all).max(1))
        .for_each(|(ds, (qi, q))| {
            ds.iter_mut().for_each(|d| d.keep = false);
            let (depth, s) = match cam {
                Some(c) => {
                    let w = cam_w(c, q.p);
                    (w as f32, (c.zoom / w) as f32)
                }
                None => (q.p[2] + zoom, zoom / (q.p[2] + zoom)),
            };
            if depth < 1.0 {
                return;
            }
            let u = q.age / q.life.max(1e-6);
            let r0 = q.size
                * curve(b.size_over_life, u)
                * s
                * scale
                * if b.cloud { CLOUD_EXTENT } else { 1.0 };
            let mut a =
                q.opacity * curve(b.opacity_over_life, u) * b.master_opacity * grad_alpha(q, b);
            if r0 <= 0.0 || a <= 0.0 {
                return;
            }
            // out of focus: every dot of every type drawn bigger, fully feathered and fainter by
            // the same rule (Camera::defocus, measured on Spheres); a sprite is blurred by as much
            // instead. The reference itself treats the types differently (ref DT2-DT6: Glow Spheres
            // stay sharp, a Star loses its rays); at the user's choice depth of field is the same
            // for all.
            let defocus = match cam {
                Some(c) if b.dof => c.defocus(depth as f64) * scale,
                _ => 0.0,
            };
            let blur = if b.ptype == 3 { 0.0 } else { defocus };
            // a Gaussian of sigma growth / sqrt 2 spreads as much (ref DT6: a sprite's blur is
            // that, sigma 0.040 x CoC, alpha kept)
            let sblur = if b.ptype == 3 {
                defocus * std::f32::consts::FRAC_1_SQRT_2
            } else {
                0.0
            };
            // The reference draws a sprite only once it's older than 0: one born on the frame doesn't show
            // (MS0, TS1-TS11), and with motion blur shows from the second sample on (MS1: 15 of
            // 16 samples). Spheres show at 0 (P1, M1).
            let unborn = |s: f32| b.ptype == 3 && !b.cloud && q.age + s <= 1e-5 * dt;
            if b.mb_samples < 2 && unborn(0.0) {
                return;
            }
            // sprites: the frame this particle shows (none once its clip has played out)
            let frame = match sprites {
                Some(sp) => {
                    let f = sprite_time(q, b, time).map(|s| clip_frame(s, dt));
                    match f.and_then(|f| sp.nearest(f)) {
                        Some(i) if !sp.frames[i].1.levels.is_empty() => i as u32,
                        _ => return,
                    }
                }
                None => u32::MAX,
            };
            let mut rgb = main_rgb(q, b);
            if let Some(c) = recolor.and_then(|c| c.get(qi)).filter(|c| c[0] >= 0.0) {
                rgb = *c;
            }
            if let Some(lt) = b.lighting.as_ref().filter(|_| !b.shade.is_empty()) {
                let k = shade(lt, &b.shade, q.p);
                rgb = [rgb[0] * k[0], rgb[1] * k[1], rgb[2] * k[2]];
            }
            let project = |m: &[[f64; 4]; 3], p: [f32; 3]| {
                let row = |r: [f64; 4]| {
                    r[0] * p[0] as f64 + r[1] * p[1] as f64 + r[2] * p[2] as f64 + r[3]
                };
                let w = row(m[2]).max(1.0);
                [
                    (row(m[0]) / w) as f32 * scale + origin[0],
                    (row(m[1]) / w) as f32 * scale + origin[1],
                ]
            };
            let screen = |p: [f32; 3]| match cam {
                Some(c) => project(&c.m, p),
                None => {
                    let s = zoom / (p[2] + zoom).max(1.0);
                    [
                        (cx + (p[0] - cx) * s) * scale + origin[0],
                        (cy + (p[1] - cy) * s) * scale + origin[1],
                    ]
                }
            };
            // Motion blur (reference: On, 360°, phase 0, 1500 px/s): the particle's shape
            // averaged over `mb_samples` copies at shutter start + k/N of the shutter, moving
            // along its velocity, so the streak reaches 15/16 of a frame's travel ahead.
            // Sprites: The reference's N samples (a sprite has detail a sphere doesn't), fewer only where
            // they'd be under half a pixel apart; each also turned by the spin over its sub-frame
            // time (not measured: The reference's Rotation Speed didn't turn sprites when set from a
            // script).
            let (mut xy, mut seg) = (screen(q.p), None);
            let mut angle = q.rot + q.spin * q.age;
            let (mut turn, mut arc) = (0.0, 0.0);
            let mut cnt = b.mb_samples as u32;
            if b.mb_samples > 1 {
                let (t0, len) = (b.shutter[0] * dt, b.shutter[1] * dt);
                // through the camera as it is at that moment of the shutter (ref CMB2, CMB3: a
                // dollying or turning camera smears still particles as this does; the reference
                // leaves them sharp when the camera only slides, CMB1, not copied)
                let at = |t: f32| {
                    let p = [
                        q.p[0] + q.v[0] * t,
                        q.p[1] + q.v[1] * t,
                        q.p[2] + q.v[2] * t,
                    ];
                    match cam.and_then(|c| c.shutter) {
                        Some([m0, m1]) => {
                            let f = ((t - t0) / len.max(1e-9)) as f64;
                            let m = std::array::from_fn(|i| {
                                std::array::from_fn(|j| m0[i][j] + (m1[i][j] - m0[i][j]) * f)
                            });
                            project(&m, p)
                        }
                        None => screen(p),
                    }
                };
                let (mut p0, p1) = (at(t0), at(t0 + len));
                // The reference's samples cover (N-1)/N of the shutter; fewer, evenly spread over the same
                // streak, give the same average while they are a third of a radius apart or closer.
                let nn = b.mb_samples as f32;
                // a newborn sprite's samples start at the first after its birth
                let mut k0 = 0;
                while k0 < b.mb_samples as u32 && unborn(t0 + len * k0 as f32 / nn) {
                    k0 += 1;
                }
                if k0 == b.mb_samples as u32 {
                    return;
                }
                let f0 = k0 as f32 / nn;
                let d = [p1[0] - p0[0], p1[1] - p0[1]];
                if k0 > 0 {
                    cnt -= k0;
                    a *= cnt as f32 / nn;
                    p0 = [p0[0] + d[0] * f0, p0[1] + d[1] * f0];
                    xy = p0;
                }
                let span = (cnt - 1) as f32;
                let sg = [d[0] * span / nn, d[1] * span / nn];
                let len_px = sg[0].hypot(sg[1]);
                if b.ptype == 3 {
                    turn = q.spin * b.shutter[1] * dt * span / nn;
                    arc = turn.abs() * r0 * sprite_diag;
                }
                if cnt > 1 && (len_px > 0.25 || arc > 0.25) {
                    xy = p0;
                    seg = Some((sg, len_px));
                }
                if b.ptype == 3 && (seg.is_some() || k0 > 0) {
                    angle += q.spin * (t0 + len * f0);
                }
            }
            let rot = {
                let (sn, cs) = angle.sin_cos();
                [cs, sn]
            };
            for (d, pt) in ds.iter_mut().zip(pattern) {
                let mut r = if b.ptype == 5 {
                    0.615 * r0 * b.streak_size * pt[2]
                } else {
                    r0
                };
                if r <= 0.0 {
                    continue;
                }
                // its feather width once grown: all of it, less a core when it's much blurred
                let (mut da, mut core, mut g) = (a, 0.0, 0.0);
                if blur > 0.0 {
                    g = blur;
                    let x = r / g;
                    da *= (x * x).min(1.0);
                    r += g;
                    // ponytail: fitted on the reference's blobs at opacity 0.04-0.32 grown 6-33 px (DOF1-DOF8),
                    // phased in over 6-10 px of growth as the reference's appear (ref renders at full size
                    // here; a draft scale shifts it)
                    core = (0.6 - 1.1 * x).max(0.0) * ((g / scale - 6.0) / 4.0).clamp(0.0, 1.0);
                    if da < 1e-4 {
                        continue;
                    }
                }
                let reach = if b.ptype == 1 {
                    r * b.glow_size.max(1.0)
                } else if b.ptype == 4 {
                    r * (1.2 * b.glow_size).max(3.0)
                } else if b.ptype == 2 {
                    r * 2.3
                } else if b.ptype == 3 {
                    // the sprite's width is 2 × Size (reference: a 200 px wide sprite layer is drawn at
                    // its own size at Size 100, half that at twice the width); reach its corners
                    r * sprite_diag
                } else {
                    r
                } + 1.0;
                let (step, n) = match seg {
                    Some((sg, len)) if b.ptype == 3 => {
                        let n = ((2.0 * len.max(arc)).ceil() as u32 + 1).clamp(2, cnt);
                        ([sg[0] / (n - 1) as f32, sg[1] / (n - 1) as f32], n)
                    }
                    Some((sg, len)) => {
                        let n = ((len / (r / 3.0).max(0.5)).ceil() as u32 + 1)
                            .clamp(2, b.mb_samples as u32);
                        ([sg[0] / (n - 1) as f32, sg[1] / (n - 1) as f32], n)
                    }
                    None => ([0.0; 2], 1),
                };
                // the sphere's feather; a Star's core has its own edge (reference: 3.1 px + 0.12·R)
                let fw = if b.ptype == 4 {
                    3.1 * scale + 0.12 * r
                } else if g > 0.0 {
                    (b.feather * r + 3.25 * scale + 2.0 * g).min(r + 0.5) - core * r
                } else {
                    (b.feather * r + 3.25 * scale).min(r + 0.5)
                };
                *d = Dot {
                    x: xy[0] + pt[0] * r0,
                    y: xy[1] + pt[1] * r0,
                    r,
                    reach,
                    w: fw,
                    rgb,
                    a: da * tiny(r / scale),
                    depth,
                    step,
                    n,
                    lut: frame,
                    keep: true,
                    rot,
                    spin: if n > 1 { turn / (n - 1) as f32 } else { 0.0 },
                    blur: sblur,
                    shadow: false,
                };
                d.reach += 3.0 * sblur;
                let bb = bbox(d);
                d.keep = bb[1] >= 0.0 && bb[0] <= w as f32 && bb[3] >= 0.0 && bb[2] <= h as f32;
            }
            // its self-shadow: a soft disk SHADOW_SIZE x its radius x Adjust Size, SHADOW_DIST x its
            // radius x Adjust Distance away from the light, drawn at its own depth so it darkens
            // the particles behind it (and its own particle when the light is behind that)
            if let (Some(sh), Some(sd)) = (shadows, ds.get_mut(m)) {
                let rw = r0 / (s * scale);
                let away = match sh_light.kind {
                    0 => sh_light.dir,
                    _ => {
                        let v = [
                            q.p[0] - sh_light.pos[0],
                            q.p[1] - sh_light.pos[1],
                            q.p[2] - sh_light.pos[2],
                        ];
                        let l = len3(v).max(1e-6);
                        [v[0] / l, v[1] / l, v[2] / l]
                    }
                };
                let k = SHADOW_DIST * sh.distance * rw;
                let sp = [
                    q.p[0] + away[0] * k,
                    q.p[1] + away[1] * k,
                    q.p[2] + away[2] * k,
                ];
                let (sdepth, ss) = match cam {
                    Some(c) => {
                        let w_ = cam_w(c, sp);
                        (w_ as f32, (c.zoom / w_) as f32)
                    }
                    None => (sp[2] + zoom, zoom / (sp[2] + zoom)),
                };
                let rs = SHADOW_SIZE * sh.size * rw * ss * scale;
                if sdepth >= 1.0 && rs > 0.05 && sh.opacity > 0.0 {
                    let xy = screen(sp);
                    *sd = Dot {
                        x: xy[0],
                        y: xy[1],
                        r: rs,
                        reach: rs + 1.0,
                        w: (sh.softness * rs).max(0.5),
                        rgb: std::array::from_fn(|i| rgb[i] + (sh.color[i] - rgb[i]) * sh.strength),
                        a: sh.opacity * a.min(1.0),
                        depth: sdepth,
                        step: [0.0; 2],
                        n: 1,
                        lut: u32::MAX,
                        keep: true,
                        rot: [1.0, 0.0],
                        spin: 0.0,
                        blur: 0.0,
                        shadow: true,
                    };
                    let bb = bbox(sd);
                    sd.keep =
                        bb[1] >= 0.0 && bb[0] <= w as f32 && bb[3] >= 0.0 && bb[2] <= h as f32;
                }
            }
        });
    sc.dots.retain(|d| d.keep);
    // blurred sprite levels for the out-of-focus sprites
    sc.sblur.clear();
    if let Some(sp) = sprites.filter(|_| b.ptype == 3) {
        let mut keys: Vec<SpriteBlurKey> = sc
            .dots
            .iter()
            .filter(|d| d.blur > 0.0)
            .filter_map(|d| sprite_blur_key(sp, d))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        sc.sblur = keys
            .into_par_iter()
            .map(|k| {
                let lv = &sp.frames[k.0 as usize].1.levels[k.1 as usize];
                (k, gauss_level(lv, 2f32.powf(k.2 as f32 / 2.0)))
            })
            .collect();
    }
    // falloff tables for the big ones, in one reused arena
    sc.big.clear();
    for (i, d) in sc.dots.iter_mut().enumerate() {
        if d.reach > LUT_REACH && b.ptype != 3 && !d.shadow {
            // (sprites keep their frame in `lut`)
            d.lut = sc.big.len() as u32;
            sc.big.push(i as u32);
        }
    }
    // split glow spheres keep two tables: the sphere's and the glow's
    let stride = if glow_split(b) {
        2 * (LUT + 1)
    } else {
        LUT + 1
    };
    sc.luts.resize(sc.big.len() * stride, 0.0);
    {
        let dots = &sc.dots;
        sc.luts
            .par_chunks_mut(stride)
            .zip(sc.big.par_iter())
            .for_each(|(t, &i)| {
                let d = &dots[i as usize];
                let r2 = d.reach * d.reach;
                let (t0, t1) = t.split_at_mut(LUT + 1);
                for (j, v) in t0.iter_mut().enumerate() {
                    let c = shape(d, b, (r2 * j as f32 / LUT as f32).sqrt());
                    *v = c[0];
                    if let Some(g) = t1.get_mut(j) {
                        *g = c[1];
                    }
                }
            });
    }
    // Drawing order. Normal: far to near, then oldest first (the key's low half is the index,
    // so equal depths keep emission order); Add, Screen and Lighten don't depend on it.
    sc.order.clear();
    sc.order.extend(0..sc.dots.len() as u64);
    if b.blend == 0 || shadows.is_some() {
        let dots = &sc.dots;
        sc.order.par_iter_mut().for_each(|k| {
            // depth > 0: its bits order like the value; far first means descending
            let far = u32::MAX - dots[*k as usize].depth.to_bits();
            *k |= (far as u64) << 32;
        });
        sc.order.par_sort_unstable();
    }
    const BAND: usize = 8;
    // Bounded: clear only the bands this pass draws in (and any gap to those cleared before),
    // growing `area`, the rectangle the host reads back
    if let Some(a) = area {
        let mut e = [w, h, 0, 0];
        for d in &sc.dots {
            let bb = bbox(d);
            e[0] = e[0].min((bb[0].floor() - 2.0).max(0.0) as usize);
            e[2] = e[2].max(((bb[1].ceil() + 2.0).max(0.0) as usize).min(w));
            e[1] = e[1].min((bb[2].floor().max(0.0) as usize) / BAND * BAND);
            e[3] = e[3].max((((bb[3].ceil().max(0.0) as usize) / BAND + 1) * BAND).min(h));
        }
        if e[0] < e[2] && e[1] < e[3] {
            let had = a[1] < a[3];
            let (y0, y1) = if had {
                (a[1].min(e[1]), a[3].max(e[3]))
            } else {
                (e[1], e[3])
            };
            let mut zero = |r: std::ops::Range<usize>| {
                if r.start < r.end {
                    px[r.start * w..r.end * w]
                        .par_iter_mut()
                        .for_each(|p| *p = [0.0; 4]);
                }
            };
            if had {
                zero(y0..a[1]);
                zero(a[3]..y1);
            } else {
                zero(y0..y1);
            }
            *a = [a[0].min(e[0]), y0, a[2].max(e[2]), y1];
        }
    }
    let nb = h.div_ceil(BAND);
    sc.bands.resize_with(nb, Vec::new);
    sc.bands.truncate(nb);
    sc.bands.iter_mut().for_each(Vec::clear);
    for &k in &sc.order {
        let i = k as u32;
        let bb = bbox(&sc.dots[i as usize]);
        let y0 = (bb[2].floor().max(0.0) as usize) / BAND;
        let y1 = ((bb[3].ceil().max(0.0) as usize) / BAND).min(nb - 1);
        for band in &mut sc.bands[y0..=y1] {
            band.push(i);
        }
    }
    // Spheres whose falloff spans their whole radius (w = R + 0.5: every small one) share one
    // shape over (d / (R + 0.5))².
    let mut small = [0.0f32; LUT + 1];
    for (j, v) in small.iter_mut().enumerate() {
        *v = 1.0 - smoothstep((j as f32 / LUT as f32).sqrt());
    }
    let ctx = Ctx {
        clear,
        sprites,
        sblur: &sc.sblur,
        dots: &sc.dots,
        luts: &sc.luts,
        stride,
        b,
        w,
        small: &small,
    };
    let bands = &sc.bands;
    let job = px
        .par_chunks_mut(w * BAND)
        .zip(bands.par_iter())
        .enumerate();
    match b.blend {
        1 => job.for_each(|(bi, (rows, list))| draw_band::<1>(&ctx, rows, bi * BAND, list)),
        2 => job.for_each(|(bi, (rows, list))| draw_band::<2>(&ctx, rows, bi * BAND, list)),
        3 => job.for_each(|(bi, (rows, list))| draw_band::<3>(&ctx, rows, bi * BAND, list)),
        _ => job.for_each(|(bi, (rows, list))| draw_band::<0>(&ctx, rows, bi * BAND, list)),
    }
}

/// The small-sphere shape S(ρ) = 1 − smoothstep(ρ) integrated along a line at distance `e` from
/// its centre (both in radii): H(e, x) = ∫ from −1 to x of S(√(e² + τ²)) dτ, on an NE × NX grid
/// over e in 0..1 and x in −1..1. A streak's average is then two lookups per pixel.
const NE: usize = 64;
const NX: usize = 129;

fn streak_table() -> &'static [f32] {
    static T: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = vec![0.0f32; NE * NX];
        const SUB: usize = 16;
        for ie in 0..NE {
            let e = ie as f64 / (NE - 1) as f64;
            let mut acc = 0.0f64;
            let s = |tau: f64| {
                let rho = (e * e + tau * tau).sqrt().min(1.0);
                1.0 - rho * rho * (3.0 - 2.0 * rho)
            };
            for ix in 1..NX {
                let (a, b) = (
                    -1.0 + 2.0 * (ix - 1) as f64 / (NX - 1) as f64,
                    -1.0 + 2.0 * ix as f64 / (NX - 1) as f64,
                );
                let hstep = (b - a) / SUB as f64;
                for k in 0..SUB {
                    acc += 0.5 * (s(a + k as f64 * hstep) + s(a + (k + 1) as f64 * hstep)) * hstep;
                }
                t[ie * NX + ix] = acc as f32;
            }
        }
        t
    })
}

#[inline(always)]
fn streak_at(t: &[f32], e: f32, x: f32) -> f32 {
    let fe = e * (NE - 1) as f32;
    let ie = (fe as usize).min(NE - 2);
    let te = fe - ie as f32;
    let fx = ((x + 1.0) * 0.5).clamp(0.0, 1.0) * (NX - 1) as f32;
    let ix = (fx as usize).min(NX - 2);
    let tx = fx - ix as f32;
    let row = |i: usize| t[i * NX + ix] + (t[i * NX + ix + 1] - t[i * NX + ix]) * tx;
    row(ie) + (row(ie + 1) - row(ie)) * te
}

struct Ctx<'a> {
    clear: bool,
    sprites: Option<&'a Sprites>,
    sblur: &'a [(SpriteBlurKey, Level)],
    dots: &'a [Dot],
    luts: &'a [f32],
    stride: usize,
    b: &'a Params,
    w: usize,
    small: &'a [f32; LUT + 1],
}

/// Composites premultiplied `rgb·a` over `o` with blend mode `M` (0 Normal, 1 Add, 2 Screen,
/// 3 Lighten).
#[inline(always)]
fn blend_px<const M: usize>(o: &mut Px, rgb: [f32; 3], a: f32) {
    blend_pm::<M>(o, [rgb[0] * a, rgb[1] * a, rgb[2] * a], a);
}

/// [`blend_px`] with the colour already premultiplied.
#[inline(always)]
fn blend_pm<const M: usize>(o: &mut Px, s: [f32; 3], a: f32) {
    match M {
        1 => {
            // Truly additive: overlapping particles keep adding light, so a dense core glows hotter
            // than its edge. The reference clamps the straight colour at 1 after each particle (its
            // 8/16 bpc behaviour), which flattens dense areas into plateaus of the particles' own
            // colour; EffectCraft hands helpers float buffers, so values above 1 survive to the
            // host (a host at 8/16 bpc clips them at its output).
            o[3] += a * (1.0 - o[3]);
            for c in 0..3 {
                o[c] += s[c];
            }
        }
        2 => {
            for c in 0..3 {
                o[c] = s[c] + o[c] - s[c] * o[c];
            }
            o[3] += a * (1.0 - o[3]);
        }
        3 => {
            for c in 0..3 {
                o[c] = o[c].max(s[c]);
            }
            o[3] = o[3].max(a);
        }
        _ => {
            let k = 1.0 - a;
            for c in 0..3 {
                o[c] = s[c] + o[c] * k;
            }
            o[3] = a + o[3] * k;
        }
    }
}

/// One pixel of a particle: its shape's coverage `cov[0]` with blend mode `M`, then its glow's
/// `cov[1]` with the glow blend mode `g` (0 Normal, 1 Add, 2 Screen).
#[inline(always)]
fn put<const M: usize>(o: &mut Px, rgb: [f32; 3], a: f32, cov: [f32; 2], g: usize) {
    let s = a * cov[0];
    if s > 1e-5 {
        blend_px::<M>(o, rgb, s);
    }
    let ga = a * cov[1];
    if ga > 1e-5 {
        match g {
            1 => blend_px::<1>(o, rgb, ga),
            2 => blend_px::<2>(o, rgb, ga),
            _ => blend_px::<0>(o, rgb, ga),
        }
    }
}

/// Linear interpolation in a falloff table indexed by `x` in 0..=LUT.
#[inline(always)]
fn lut_at(t: &[f32; LUT + 1], x: f32) -> f32 {
    let j = (x as usize).min(LUT - 1);
    t[j] + (t[j + 1] - t[j]) * (x - j as f32)
}

/// One row of a still particle: `cov(d²)` for the pixels within `reach²`.
#[inline(always)]
fn still_row<const M: usize>(
    row: &mut [Px],
    d: &Dot,
    py: f32,
    reach2: f32,
    g: usize,
    cov: impl Fn(f32) -> [f32; 2],
) {
    let dy = py - d.y;
    let dy2 = dy * dy;
    let half = (reach2 - dy2).max(0.0).sqrt();
    let x0 = (d.x - half).floor().max(0.0) as usize;
    let x1 = ((d.x + half).ceil().max(0.0) as usize).min(row.len());
    for (x, o) in row.iter_mut().enumerate().take(x1).skip(x0) {
        let dx = x as f32 + 0.5 - d.x;
        let d2 = dx * dx + dy2;
        if d2 < reach2 {
            put::<M>(o, d.rgb, d.a, cov(d2), g);
        }
    }
}

/// A sprite particle's frame mapping: its level about a screen pixel's size, that level's scale
/// `k`, sprite pixels per screen pixel `inv`, the particle centre in the frame's pixels (`cu`,
/// `cv`: the centre of the box around all frames) and the half extents on screen (`ex`, `ey`,
/// one sprite pixel wider for the bilinear edge).
struct SpriteMap<'a> {
    level: &'a Level,
    k: f32,
    inv: f32,
    cu: f32,
    cv: f32,
    ex: f32,
    ey: f32,
}

fn sprite_map<'a>(sp: &'a Sprites, sblur: &'a [(SpriteBlurKey, Level)], d: &Dot) -> SpriteMap<'a> {
    let fr = &sp.frames[d.lut as usize].1;
    let (bw, bh) = (sp.bx[2] - sp.bx[0], sp.bx[3] - sp.bx[1]);
    let inv = bw / (2.0 * d.r).max(1e-6);
    let lvl = (inv.max(1.0).log2().floor() as usize).min(fr.levels.len() - 1);
    let mut m = SpriteMap {
        level: &fr.levels[lvl],
        k: 1.0 / (1u32 << lvl) as f32,
        inv,
        cu: sp.bx[0] + 0.5 * bw - fr.x0,
        cv: sp.bx[1] + 0.5 * bh - fr.y0,
        ex: (0.5 * bw + 1.0) / inv,
        ey: (0.5 * bh + 1.0) / inv,
    };
    // out of focus: its blurred level, which has a margin of 3 sigma around the frame
    if let Some((key, lv)) = (d.blur > 0.0)
        .then(|| sprite_blur_key(sp, d))
        .flatten()
        .and_then(|key| {
            let i = sblur.binary_search_by_key(&key, |e| e.0).ok()?;
            Some((key, &sblur[i].1))
        })
    {
        let k = 1.0 / (1u32 << key.1) as f32;
        let margin = gauss_margin(2f32.powf(key.2 as f32 / 2.0)) as f32;
        m.level = lv;
        m.k = k;
        m.cu += margin / k;
        m.cv += margin / k;
        m.ex += 3.0 * d.blur;
        m.ey += 3.0 * d.blur;
    }
    m
}

/// Which blurred sprite level an out-of-focus sprite dot draws from: its frame, the mip level
/// (the dot's own, or coarser so the blur is at most 4 of its pixels), and the blur's sigma in
/// that level's pixels as 2 log2 sigma, rounded (sqrt 2 steps). None: under a third of a pixel.
type SpriteBlurKey = (u32, u32, i32);

fn sprite_blur_key(sp: &Sprites, d: &Dot) -> Option<SpriteBlurKey> {
    let fr = &sp.frames[d.lut as usize].1;
    let bw = sp.bx[2] - sp.bx[0];
    let inv = bw / (2.0 * d.r).max(1e-6);
    let s0 = d.blur * inv;
    let own = inv.max(1.0).log2().floor() as usize;
    let lvl = own
        .max((s0 / 4.0).max(1.0).log2().ceil() as usize)
        .min(fr.levels.len() - 1);
    let s = s0 / (1u32 << lvl) as f32;
    (s >= 0.33).then(|| (d.lut, lvl as u32, (2.0 * s.log2()).round() as i32))
}

fn gauss_margin(sigma: f32) -> usize {
    (3.0 * sigma).ceil() as usize
}

/// `lv` blurred by a Gaussian of `sigma` pixels (premultiplied, so alpha and colour spread
/// alike), with a margin of 3 sigma on each side for what spreads out.
fn gauss_level(lv: &Level, sigma: f32) -> Level {
    let m = gauss_margin(sigma);
    let r = m as i64;
    let kern: Vec<f32> = (-r..=r)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = kern.iter().sum();
    let kern: Vec<f32> = kern.iter().map(|k| k / sum).collect();
    let (w, h) = (lv.w, lv.h);
    let (ow, oh) = (w + 2 * m, h + 2 * m);
    // across: source rows into the wide grid
    let mut tmp = vec![[0.0f32; 4]; ow * h];
    for y in 0..h {
        for x in 0..w {
            let p = lv.at(x, y);
            if p[3] <= 0.0 && p[0] <= 0.0 {
                continue;
            }
            for (j, k) in kern.iter().enumerate() {
                let o = &mut tmp[y * ow + x + j];
                for c in 0..4 {
                    o[c] += p[c] * k;
                }
            }
        }
    }
    // down
    let mut out = vec![[0.0f32; 4]; ow * oh];
    for y in 0..h {
        for x in 0..ow {
            let p = tmp[y * ow + x];
            if p[3] <= 0.0 && p[0] <= 0.0 {
                continue;
            }
            for (j, k) in kern.iter().enumerate() {
                let o = &mut out[(y + j) * ow + x];
                for c in 0..4 {
                    o[c] += p[c] * k;
                }
            }
        }
    }
    Level::new(ow, oh, &out)
}

/// The dx range of a row `dy` below the centre inside the sprite rectangle turned by (cs, sn):
/// |dx·cs + dy·sn| <= ex and |-dx·sn + dy·cs| <= ey.
#[inline(always)]
fn rect_span(cs: f32, sn: f32, dy: f32, ex: f32, ey: f32) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
    for (a, b, e) in [(cs, dy * sn, ex), (-sn, dy * cs, ey)] {
        if a.abs() > 1e-6 {
            let (p, q) = ((-e - b) / a, (e - b) / a);
            lo = lo.max(p.min(q));
            hi = hi.min(p.max(q));
        } else if b.abs() > e {
            hi = f32::NEG_INFINITY;
        }
    }
    (lo, hi)
}

/// Composites sprite texel `t` (premultiplied) at opacity `a` in Sprite Color `mode`.
#[inline(always)]
fn sprite_put<const M: usize>(o: &mut Px, t: Px, mode: usize, tint: [f32; 3], a: f32) {
    if t[3] <= 1e-5 {
        return;
    }
    let rgb = match mode {
        // Colorize: the sprite's luminance tints the particle colour
        1 => {
            let lum = 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
            [tint[0] * lum, tint[1] * lum, tint[2] * lum]
        }
        // Fill: the particle colour through the sprite's alpha
        2 => [tint[0] * t[3], tint[1] * t[3], tint[2] * t[3]],
        _ => [t[0], t[1], t[2]],
    };
    blend_pm::<M>(o, [rgb[0] * a, rgb[1] * a, rgb[2] * a], t[3] * a);
}

/// One sprite particle over rows `y0..y1` of the band: each pixel maps back through the rotation
/// and scale into the sprite level whose pixels are about the size of a screen pixel. Each row
/// visits only the span inside the rotated sprite rectangle, stepping the sprite coordinates.
#[allow(clippy::too_many_arguments)]
fn sprite_rows<const M: usize>(
    c: &Ctx,
    sp: &Sprites,
    d: &Dot,
    rows: &mut [Px],
    y_base: usize,
    y0: usize,
    y1: usize,
) {
    let w = c.w;
    let m = sprite_map(sp, c.sblur, d);
    let (inv, k) = (m.inv, m.k);
    let (cs, sn) = (d.rot[0], d.rot[1]);
    // sprite coordinates (in this level) per screen pixel step in x
    let (du, dv) = (cs * inv * k, -sn * inv * k);
    for y in y0..y1 {
        let dy = y as f32 + 0.5 - d.y;
        let (lo, hi) = rect_span(cs, sn, dy, m.ex, m.ey);
        let x0 = (d.x + lo).floor().max(0.0) as usize;
        let x1 = ((d.x + hi).ceil().max(0.0) as usize).min(w);
        if x0 >= x1 {
            continue;
        }
        let row = &mut rows[(y - y_base) * w..(y - y_base + 1) * w];
        let dx = x0 as f32 + 0.5 - d.x;
        // screen offset rotated back (Rotate Z is clockwise on screen) and scaled to the sprite
        let mut u = ((dx * cs + dy * sn) * inv + m.cu) * k;
        let mut v = ((-dx * sn + dy * cs) * inv + m.cv) * k;
        for o in &mut row[x0..x1] {
            let t = m.level.sample(u, v);
            u += du;
            v += dv;
            sprite_put::<M>(o, t, c.b.sprite_mode, d.rgb, d.a);
        }
    }
}

/// A motion-blurred sprite particle: the average of its `n` sub-frame samples (each moved by
/// `step` and turned by `spin`), composited once, like the reference's sub-frame copies.
#[allow(clippy::too_many_arguments)]
fn sprite_rows_blur<const M: usize>(
    c: &Ctx,
    sp: &Sprites,
    d: &Dot,
    rows: &mut [Px],
    y_base: usize,
    y0: usize,
    y1: usize,
) {
    let w = c.w;
    let m = sprite_map(sp, c.sblur, d);
    let (inv, k) = (m.inv, m.k);
    let n = (d.n as usize).min(64);
    let a0 = d.rot[1].atan2(d.rot[0]);
    // per sample: centre x, y, cos, sin
    let mut smp = [[0.0f32; 4]; 64];
    for (i, s) in smp.iter_mut().enumerate().take(n) {
        let (sn, cs) = (a0 + d.spin * i as f32).sin_cos();
        *s = [
            d.x + d.step[0] * i as f32,
            d.y + d.step[1] * i as f32,
            cs,
            sn,
        ];
    }
    let smp = &smp[..n];
    let wgt = 1.0 / n as f32;
    ACC.with_borrow_mut(|acc| {
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            // each sample's span of this row, and the row's span over all of them
            let mut spans = [(0usize, 0usize); 64];
            let (mut x0, mut x1) = (usize::MAX, 0);
            for (s, sp_) in smp.iter().zip(spans.iter_mut()) {
                let (lo, hi) = rect_span(s[2], s[3], py - s[1], m.ex, m.ey);
                let a = (s[0] + lo).floor().max(0.0) as usize;
                let b = ((s[0] + hi).ceil().max(0.0) as usize).min(w);
                if a < b {
                    *sp_ = (a, b);
                    x0 = x0.min(a);
                    x1 = x1.max(b);
                }
            }
            if x0 >= x1 {
                continue;
            }
            acc.clear();
            acc.resize(x1 - x0, [0.0; 4]);
            for (s, &(a, b)) in smp.iter().zip(&spans[..n]) {
                if a >= b {
                    continue;
                }
                let (dx, dy, cs, sn) = (a as f32 + 0.5 - s[0], py - s[1], s[2], s[3]);
                let mut u = ((dx * cs + dy * sn) * inv + m.cu) * k;
                let mut v = ((-dx * sn + dy * cs) * inv + m.cv) * k;
                let (du, dv) = (cs * inv * k, -sn * inv * k);
                for o in &mut acc[a - x0..b - x0] {
                    let t = m.level.sample(u, v);
                    u += du;
                    v += dv;
                    for (o, t) in o.iter_mut().zip(t) {
                        *o += t;
                    }
                }
            }
            let row = &mut rows[(y - y_base) * w + x0..(y - y_base) * w + x1];
            for (o, t) in row.iter_mut().zip(acc.iter()) {
                sprite_put::<M>(o, t.map(|v| v * wgt), c.b.sprite_mode, d.rgb, d.a);
            }
        }
    });
}

/// A self-shadow over rows `y0..y1`: a disk flat inside, then a smoothstep to nothing over its soft
/// edge, darkening the colour already drawn towards its own (alpha kept: The reference draws no self-shadows
/// on the background, SH3, SH4).
fn shadow_rows(c: &Ctx, d: &Dot, rows: &mut [Px], y_base: usize, y0: usize, y1: usize) {
    let w = c.w;
    let inner = (d.r - d.w).max(0.0);
    let inv = 1.0 / (d.r - inner).max(1e-6);
    for y in y0..y1 {
        let dy = y as f32 + 0.5 - d.y;
        if dy.abs() >= d.r {
            continue;
        }
        let half = (d.r * d.r - dy * dy).sqrt();
        let x0 = (d.x - half).floor().max(0.0) as usize;
        let x1 = ((d.x + half).ceil().max(0.0) as usize).min(w);
        let row = &mut rows[(y - y_base) * w..(y - y_base + 1) * w];
        for (x, o) in row.iter_mut().enumerate().take(x1).skip(x0) {
            if o[3] <= 0.0 {
                continue;
            }
            let dx = x as f32 + 0.5 - d.x;
            let t = (((dx * dx + dy * dy).sqrt() - inner) * inv).clamp(0.0, 1.0);
            let k = d.a * (1.0 - t * t * (3.0 - 2.0 * t));
            for i in 0..3 {
                o[i] += (d.rgb[i] * o[3] - o[i]) * k;
            }
        }
    }
}

/// Where a camera matrix (rows: x, y and depth of a point, projected as x / depth, y / depth)
/// looks from: the point every row sends to 0.
fn camera_eye(m: &[[f64; 4]; 3]) -> [f32; 3] {
    let a = |i: usize, j: usize| m[i][j];
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1))
        - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-12 {
        return [0.0; 3];
    }
    let bv = [-m[0][3], -m[1][3], -m[2][3]];
    let col = |k: usize| {
        let mut mm = [
            [m[0][0], m[0][1], m[0][2]],
            [m[1][0], m[1][1], m[1][2]],
            [m[2][0], m[2][1], m[2][2]],
        ];
        for (i, row) in mm.iter_mut().enumerate() {
            row[k] = bv[i];
        }
        mm[0][0] * (mm[1][1] * mm[2][2] - mm[1][2] * mm[2][1])
            - mm[0][1] * (mm[1][0] * mm[2][2] - mm[1][2] * mm[2][0])
            + mm[0][2] * (mm[1][0] * mm[2][1] - mm[1][1] * mm[2][0])
    };
    [
        (col(0) / det) as f32,
        (col(1) / det) as f32,
        (col(2) / det) as f32,
    ]
}

thread_local! {
    /// a row of summed motion blur samples (sprites)
    static ACC: std::cell::RefCell<Vec<Px>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn draw_band<const M: usize>(c: &Ctx, rows: &mut [Px], y_base: usize, list: &[u32]) {
    if c.clear {
        rows.fill([0.0; 4]);
    }
    let w = c.w;
    let y_end = y_base + rows.len() / w;
    for &i in list {
        let d = &c.dots[i as usize];
        let (ex, ey) = (d.step[0] * (d.n - 1) as f32, d.step[1] * (d.n - 1) as f32);
        let y0 = ((d.y.min(d.y + ey) - d.reach).floor().max(y_base as f32) as usize).max(y_base);
        let y1 = ((d.y.max(d.y + ey) + d.reach).ceil().max(0.0) as usize).min(y_end);
        let reach2 = d.reach * d.reach;
        if d.shadow {
            shadow_rows(c, d, rows, y_base, y0, y1);
            continue;
        }
        if let Some(sp) = c.sprites {
            if d.n > 1 {
                sprite_rows_blur::<M>(c, sp, d, rows, y_base, y0, y1);
            } else {
                sprite_rows::<M>(c, sp, d, rows, y_base, y0, y1);
            }
            continue;
        }
        // the falloff table, and the glow's for glow spheres
        let tables: Option<(&[f32; LUT + 1], Option<&[f32; LUT + 1]>)> =
            (d.lut != u32::MAX).then(|| {
                let t = &c.luts[d.lut as usize * c.stride..(d.lut as usize + 1) * c.stride];
                let (t0, t1) = t.split_at(LUT + 1);
                (t0.try_into().unwrap(), t1.try_into().ok())
            });
        let gm = c.b.glow_blend;
        if c.b.ptype == 4 {
            star_rows::<M>(c, d, tables.map(|t| t.0), rows, y_base, y0, y1);
            continue;
        }
        // the shared small-sphere shape applies when the falloff spans the radius
        let edge = d.r + 0.5;
        let small = matches!(c.b.ptype, 0 | 5) && d.w >= edge;
        let k_lut = LUT as f32 / reach2;
        let e2 = edge * edge;
        let k_small = LUT as f32 / e2;
        if d.n == 1 {
            for y in y0..y1 {
                let row = &mut rows[(y - y_base) * w..(y - y_base + 1) * w];
                let py = y as f32 + 0.5;
                match tables {
                    // the sphere ends at its edge; the glow reaches Glow Size times further
                    Some((t, Some(tg))) => still_row::<M>(row, d, py, reach2, gm, |d2| {
                        let s = if d2 < e2 { lut_at(t, d2 * k_lut) } else { 0.0 };
                        [s, lut_at(tg, d2 * k_lut)]
                    }),
                    Some((t, None)) => {
                        still_row::<M>(row, d, py, reach2, gm, |d2| [lut_at(t, d2 * k_lut), 0.0])
                    }
                    None if small => still_row::<M>(row, d, py, reach2, gm, |d2| {
                        if d2 * k_small < LUT as f32 {
                            [lut_at(c.small, d2 * k_small), 0.0]
                        } else {
                            [0.0; 2]
                        }
                    }),
                    None => still_row::<M>(row, d, py, reach2, gm, |d2| shape(d, c.b, d2.sqrt())),
                }
            }
            continue;
        }
        // Motion blur: the average over the streak's samples; per pixel only those within reach.
        let cov_at = |d2: f32| match tables {
            Some((t, tg)) => [
                lut_at(t, d2 * k_lut),
                tg.map_or(0.0, |tg| lut_at(tg, d2 * k_lut)),
            ],
            None if small => {
                if d2 * k_small < LUT as f32 {
                    [lut_at(c.small, d2 * k_small), 0.0]
                } else {
                    [0.0; 2]
                }
            }
            None => shape(d, c.b, d2.sqrt()),
        };
        let s2 = d.step[0] * d.step[0] + d.step[1] * d.step[1];
        let sp = s2.sqrt();
        let span = d.reach / sp;
        let inv_n = 1.0 / d.n as f32;
        // Small spheres with samples under half a radius apart: the samples' sum is the shape
        // integrated along the streak (midpoint rule), from the shared table.
        let smooth = small && sp <= 0.5 * edge;
        let (dir, len, inv_edge, k_streak) = (
            [d.step[0] / sp, d.step[1] / sp],
            sp * (d.n - 1) as f32,
            1.0 / edge,
            edge / (sp * d.n as f32),
        );
        let ht = streak_table();
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            // the part of the streak within reach of this row, widened by the reach
            let (sa, sb) = if ey.abs() > 1e-6 {
                let (u, v) = ((py - d.reach - d.y) / ey, (py + d.reach - d.y) / ey);
                (u.min(v).max(0.0), u.max(v).min(1.0))
            } else {
                (0.0, 1.0)
            };
            if sa > sb {
                continue;
            }
            let (xs, xe) = (d.x + ex * sa, d.x + ex * sb);
            let x0 = (xs.min(xe) - d.reach).floor().max(0.0) as usize;
            let x1 = ((xs.max(xe) + d.reach).ceil().max(0.0) as usize).min(w);
            let row = &mut rows[(y - y_base) * w..(y - y_base + 1) * w];
            let ry = py - d.y;
            if smooth {
                for (x, o) in row.iter_mut().enumerate().take(x1).skip(x0) {
                    let rx = x as f32 + 0.5 - d.x;
                    let along = rx * dir[0] + ry * dir[1];
                    let e = (rx * dir[1] - ry * dir[0]).abs() * inv_edge;
                    if e >= 1.0 {
                        continue;
                    }
                    let hi = streak_at(ht, e, (along + 0.5 * sp) * inv_edge);
                    let lo = streak_at(ht, e, (along - len - 0.5 * sp) * inv_edge);
                    let a = d.a * (hi - lo) * k_streak;
                    if a > 1e-5 {
                        blend_px::<M>(o, d.rgb, a);
                    }
                }
                continue;
            }
            for (x, o) in row.iter_mut().enumerate().take(x1).skip(x0) {
                let rx = x as f32 + 0.5 - d.x;
                let t = (rx * d.step[0] + ry * d.step[1]) / s2;
                let k0 = (t - span).ceil().max(0.0) as i64;
                let k1 = ((t + span).floor() as i64).min(d.n as i64 - 1);
                let mut cov = [0.0; 2];
                for k in k0..=k1 {
                    let (dx, dy) = (rx - d.step[0] * k as f32, ry - d.step[1] * k as f32);
                    let d2 = dx * dx + dy * dy;
                    if d2 < reach2 {
                        let c = cov_at(d2);
                        cov[0] += c[0];
                        cov[1] += c[1];
                    }
                }
                put::<M>(o, d.rgb, d.a * inv_n, cov, gm);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Vec<f64> {
        let mut v = vec![];
        for p in DEFS {
            match &p.kind {
                Slider(x, ..) | Angle(x) => v.push(*x),
                Check(b) => v.push(*b as u8 as f64),
                Popup(_, i) => v.push(*i as f64),
                Point3(p) => v.extend(p),
                Color(c) => v.extend(c),
                Layer => v.push(-1.0),
                Text(_) | Gradient(_) => {}
            }
        }
        v
    }

    fn set(v: &mut [f64], id: &str, x: &[f64]) {
        let mut at = 0;
        for p in DEFS {
            if p.id == id {
                v[at..at + x.len()].copy_from_slice(x);
                return;
            }
            at += slots(&p.kind);
        }
        panic!("no {id}");
    }

    fn run(flat: &[f64], frames: u64) -> State {
        let p = Params::from_flat(flat);
        let mut st = State::default();
        for _ in 0..frames {
            step(&mut st, &p, &p, 1.0 / 30.0);
        }
        st
    }

    /// Depth of field grows a particle by CoC x (0.050 + 0.006 u): ref DOF1 (focus 2667,
    /// aperture 200, zoom 2667) 16.7 px at depth 1067, DOF3 (focus 1600) 14.4 px at 5667; none
    /// in focus or with the camera's depth of field off.
    /// Bounded drawing (EffectCraft's `frame_bounds`): whatever the buffer held, the pixels inside
    /// the returned bounds are the full drawing's, and the full drawing is transparent outside
    /// them (spheres with aux trails and motion blur, glow, puffs, a burst off to one side).
    #[test]
    fn bounded_drawing_matches_the_full_frame() {
        let (w, h) = (320, 200);
        let mut f = defaults();
        for (id, v) in [
            ("emitter_behavior", 1.0),
            ("rate", 400.0),
            ("velocity", 120.0),
            ("size", 5.0),
            ("motion_blur", 1.0),
            ("aux_emit", 1.0),
            ("aux_rate", 30.0),
        ] {
            set(&mut f, id, &[v]);
        }
        set(&mut f, "position", &[90.0, 70.0, 0.0]);
        let scenes = [
            (f.clone(), 0.0),
            (
                {
                    let mut g = f.clone();
                    set(&mut g, "particle_type", &[1.0]);
                    g
                },
                1.0,
            ),
            (
                {
                    let mut g = f.clone();
                    set(&mut g, "particle_type", &[2.0]);
                    set(&mut g, "size", &[30.0]);
                    g
                },
                0.0,
            ),
        ];
        for (k, (g, _)) in scenes.iter().enumerate() {
            let st = run(g, 12);
            let p = Params::from_flat(g);
            let draw = |px: &mut [Px], bounded: bool| {
                render(
                    px,
                    w,
                    h,
                    &st,
                    &p,
                    1.0,
                    [0.0; 2],
                    [w as f32, h as f32],
                    1.0 / 30.0,
                    &mut Scratch::default(),
                    &Sprites::default(),
                    None,
                    None,
                    bounded,
                )
            };
            let mut full = vec![[0.0f32; 4]; w * h];
            draw(&mut full, false);
            let mut part = vec![[7.0f32; 4]; w * h];
            let b = draw(&mut part, true).map(|v| v as usize);
            assert!(
                b[0] < b[2] && b[1] < b[3] && (b[2] - b[0]) * (b[3] - b[1]) < w * h,
                "scene {k}: {b:?}"
            );
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if (b[0]..b[2]).contains(&x) && (b[1]..b[3]).contains(&y) {
                        assert_eq!(part[i], full[i], "scene {k} at {x},{y}");
                    } else {
                        assert_eq!(
                            full[i], [0.0; 4],
                            "scene {k}: drawn outside the bounds at {x},{y}"
                        );
                    }
                }
            }
        }
    }

    /// Color over Life from EffectCraft's gradient JSON: colours and opacities between stops, the
    /// ends held, stops sorted; without one, Color to Gradient End Color as before.
    #[test]
    fn color_over_life_gradient() {
        let g = Grad::from_json(
            r#"{"colors":[[1.0,[0,0,1,1]],[0.0,[1,0,0,1]],[0.5,[0,1,0,1]]],"opacities":[[0.5,1],[1.0,0]]}"#,
        )
        .unwrap();
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5);
        assert!(near(g.color(0.0), [1.0, 0.0, 0.0]) && near(g.color(-1.0), [1.0, 0.0, 0.0]));
        assert!(near(g.color(0.25), [0.5, 0.5, 0.0]) && near(g.color(0.75), [0.0, 0.5, 0.5]));
        assert!((g.alpha(0.2) - 1.0).abs() < 1e-6 && (g.alpha(0.75) - 0.5).abs() < 1e-6);
        assert!(Grad::from_json(r#"{"colors":[]}"#).is_none() && Grad::from_json("x").is_none());
        let q = Particle {
            age: 1.0,
            life: 4.0,
            grad: 0.75,
            ..Particle::default()
        };
        let mut b = Params::from_flat(&defaults());
        b.set_color = 1;
        assert!(near(main_rgb(&q, &b), lerp3(b.color, b.end_color, 0.25)));
        b.gradient = Some(std::sync::Arc::new(g));
        assert!(near(main_rgb(&q, &b), [0.5, 0.5, 0.0]) && grad_alpha(&q, &b) == 1.0);
        b.set_color = 2;
        assert!(near(main_rgb(&q, &b), [0.0, 0.5, 0.5]) && (grad_alpha(&q, &b) - 0.5).abs() < 1e-6);
    }

    /// Position Subframe on a circle of radius 300 at 1000 px a frame: the path keeps to the
    /// circle (10x Smooth within 0.3 px, 10x Linear within the 4.2 px sagitta of a tenth of the
    /// arc), meets the frames exactly, and its speed is the emitter's (within 4%) and the same
    /// either side of a frame.
    #[test]
    fn subframe_path_follows_a_circle() {
        let (r, w, dt) = (300.0f32, 1000.0 / 300.0, 1.0 / 30.0);
        let at = |s: f32| [r * (w * s).cos(), r * (w * s).sin(), 0.0];
        let frame = |k: f32, mode| Params {
            pos: at(k),
            subframe: mode,
            sub_pos: (1..10).map(|i| at(k - 1.0 + i as f32 / 10.0)).collect(),
            ..Params::default()
        };
        let d = |p: [f32; 3], q: [f32; 3]| len3([p[0] - q[0], p[1] - q[1], p[2] - q[2]]);
        for (mode, tol) in [(1, 4.2), (2, 0.3)] {
            let (a, b, c) = (frame(0.0, mode), frame(1.0, mode), frame(2.0, mode));
            for (a, b) in [(&a, &b), (&b, &c)] {
                for k in 0..=100 {
                    let frac = k as f32 / 100.0;
                    let (p, v) = emitter_path(a, b, frac, dt);
                    assert!((len3(p) - r).abs() < tol, "mode {mode} frac {frac}: {p:?}");
                    let dv = (len3(v) * dt / (r * w) - 1.0).abs();
                    assert!(dv < 0.04, "mode {mode} frac {frac}: {v:?}");
                }
                assert!(d(emitter_path(a, b, 1.0, dt).0, a.pos) < 1e-3);
                assert!(d(emitter_path(a, b, 0.0, dt).0, b.pos) < 1e-3);
            }
            let (v0, v1) = (
                emitter_path(&a, &b, 0.0, dt).1,
                emitter_path(&b, &c, 1.0, dt).1,
            );
            assert!(d(v0, v1) < 1e-3 * len3(v0), "{v0:?} {v1:?}");
        }
        // Linear (no samples): the chord, half a radius inside the circle at its middle
        let (a, b) = (
            frame(0.0, 0),
            Params {
                pos: at(1.0),
                ..Params::default()
            },
        );
        let mid = len3(emitter_path(&a, &b, 0.5, dt).0);
        assert!(mid < 0.2 * r, "{mid}");
    }

    /// The Sphere grid: the cube's corners land on the sphere, its centre stays, and every cell
    /// of an 8^3 grid is inside the ball.
    #[test]
    fn cube_to_ball_fills_the_ball() {
        let r = |p: [f32; 3]| len3(cube_to_ball(p));
        assert!((r([1.0, 1.0, 1.0]) - 1.0).abs() < 1e-5 && (r([1.0, 0.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cube_to_ball([0.0; 3]), [0.0; 3]);
        let c = |i: u32| 2.0 * (i as f32 + 0.5) / 8.0 - 1.0;
        for i in 0..512 {
            assert!(r([c(i % 8), c(i / 8 % 8), c(i / 64)]) < 1.0);
        }
    }

    #[test]
    fn defocus_grows_as_in_ae() {
        let cam = |dof| Camera {
            m: [[0.0; 4]; 3],
            zoom: 2666.67,
            dof,
            shutter: None,
        };
        let a = cam(Some([2666.67, 200.0, 1.0]));
        assert!(
            (a.defocus(1066.67) - 16.7).abs() < 0.2,
            "{}",
            a.defocus(1066.67)
        );
        assert!(a.defocus(2666.67).abs() < 1e-3);
        let b = cam(Some([1600.0, 200.0, 1.0]));
        assert!(
            (b.defocus(5666.67) - 14.4).abs() < 0.2,
            "{}",
            b.defocus(5666.67)
        );
        assert_eq!(cam(None).defocus(1066.67), 0.0);
    }

    #[test]
    fn manifest_matches_flat_layout() {
        assert_eq!(defaults().len(), nflat());
        assert_eq!(
            manifest(true)["params"].as_array().unwrap().len(),
            DEFS.len()
        );
        let gradients = DEFS
            .iter()
            .filter(|p| matches!(p.kind, Gradient(_)))
            .count();
        assert_eq!(
            manifest(false)["params"].as_array().unwrap().len(),
            DEFS.len() - gradients
        );
        let name = |g: bool, id: &str| {
            let m = manifest(g);
            m["params"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == id)
                .unwrap()["name"]
                .clone()
        };
        assert_eq!(name(false, "end_color"), "Gradient End Color");
        assert_eq!(name(true, "aux_end_color"), "End Color (stock EC only)");
    }

    /// The reference line test (P1): Directional along +x, 500 px/s, 30/s, no drag: one particle per
    /// frame, 16.7 px apart, the newest at the emitter and the oldest 3 s old.
    #[test]
    fn stream_matches_reference() {
        let mut f = defaults();
        set(&mut f, "position", &[100.0, 540.0, 0.0]);
        set(&mut f, "direction", &[1.0]);
        set(&mut f, "direction_spread", &[0.0]);
        set(&mut f, "y_rotation", &[90.0]);
        set(&mut f, "velocity", &[500.0]);
        set(&mut f, "velocity_random", &[0.0]);
        set(&mut f, "air_resistance", &[0.0]);
        set(&mut f, "rate", &[30.0]);
        let st = run(&f, 105);
        let mut xs: Vec<f32> = st.ps.iter().map(|q| q.p[0]).collect();
        xs.sort_by(f32::total_cmp);
        assert_eq!(xs.len(), 91);
        assert!(
            (xs[0] - 100.0).abs() < 0.01 && (xs[90] - 1600.0).abs() < 0.1,
            "{} {}",
            xs[0],
            xs[90]
        );
        assert!(
            st.ps
                .iter()
                .all(|q| (q.p[1] - 540.0).abs() < 1e-3 && q.p[2].abs() < 1e-3)
        );
    }

    /// A moving emitter lays an even trail: 100 births a frame while it moves 50 px a frame, no
    /// forces, so the particles sit 0.5 px apart along the path, across frame boundaries too.
    #[test]
    fn moving_emitter_trail_is_even() {
        let mut f = defaults();
        set(&mut f, "velocity", &[0.0]);
        set(&mut f, "velocity_random", &[0.0]);
        set(&mut f, "velocity_from_motion", &[0.0]);
        set(&mut f, "air_resistance", &[0.0]);
        set(&mut f, "rate", &[3000.0]);
        set(&mut f, "life", &[1.0]);
        let at = |x: f64| {
            let mut g = f.clone();
            set(&mut g, "position", &[x, 540.0, 0.0]);
            Params::from_flat(&g)
        };
        let mut st = State::default();
        for k in 0..20 {
            step(
                &mut st,
                &at(50.0 * k as f64),
                &at(50.0 * (k + 1) as f64),
                1.0 / 30.0,
            );
        }
        let mut xs: Vec<f32> = st.ps.iter().map(|q| q.p[0]).collect();
        xs.sort_by(f32::total_cmp);
        let gaps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        let worst = gaps.iter().cloned().fold(0.0f32, f32::max);
        assert!(worst < 1.0, "largest gap {worst} px (expected about 0.5)");
    }

    /// The Light(s) emitter lays an even trail behind a moving light, as a moving emitter does:
    /// the light moves 50 px a frame, 100 births a frame land 0.5 px apart along its path.
    #[test]
    fn moving_light_trail_is_even() {
        let mut f = defaults();
        set(&mut f, "emitter_type", &[4.0]);
        set(&mut f, "emitter_size_x", &[0.0]);
        set(&mut f, "emitter_size_y", &[0.0]);
        set(&mut f, "emitter_size_z", &[0.0]);
        set(&mut f, "velocity", &[0.0]);
        set(&mut f, "velocity_random", &[0.0]);
        set(&mut f, "air_resistance", &[0.0]);
        set(&mut f, "rate", &[3000.0]);
        set(&mut f, "life", &[1.0]);
        let at = |x: f32| {
            let mut p = Params::from_flat(&f);
            p.lights = vec![EmitLight {
                pos: [x, 540.0, 0.0],
                intensity: 1.0,
            }];
            p
        };
        let mut st = State::default();
        for k in 0..20 {
            step(
                &mut st,
                &at(50.0 * k as f32),
                &at(50.0 * (k + 1) as f32),
                1.0 / 30.0,
            );
        }
        let mut xs: Vec<f32> = st.ps.iter().map(|q| q.p[0]).collect();
        xs.sort_by(f32::total_cmp);
        let worst = xs.windows(2).map(|w| w[1] - w[0]).fold(0.0f32, f32::max);
        assert!(
            xs.len() > 1000 && worst < 1.0,
            "largest gap {worst} px over {} particles",
            xs.len()
        );
    }

    /// A fast stream under air resistance stays continuous: the particles born within a frame are
    /// spaced like their neighbours across the frame boundaries, with no gap where one frame's
    /// births meet the last's (a straight-line first step left ~90 px gaps here).
    #[test]
    fn stream_has_no_gaps_at_frame_boundaries() {
        let mut f = defaults();
        set(&mut f, "position", &[100.0, 540.0, 0.0]);
        set(&mut f, "direction", &[1.0]);
        set(&mut f, "direction_spread", &[0.0]);
        set(&mut f, "y_rotation", &[90.0]);
        set(&mut f, "velocity", &[1300.0]);
        set(&mut f, "velocity_random", &[0.0]);
        set(&mut f, "air_resistance", &[0.5]);
        set(&mut f, "rate", &[300.0]);
        set(&mut f, "life", &[1.0]);
        let st = run(&f, 20);
        let mut xs: Vec<f32> = st.ps.iter().map(|q| q.p[0]).collect();
        xs.sort_by(f32::total_cmp);
        let gaps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.len() > 100);
        // spacing shrinks smoothly with age as drag slows the stream: no gap far above its neighbours
        for i in 2..gaps.len() - 2 {
            let around = (gaps[i - 2] + gaps[i - 1] + gaps[i + 1] + gaps[i + 2]) / 4.0;
            assert!(
                gaps[i] < 1.5 * around + 0.05,
                "gap {i}: {} px, neighbours {around} px",
                gaps[i]
            );
        }
    }

    /// Gravity 100 without drag falls 100 px in the first second (ref P4 row 3).
    #[test]
    fn gravity_matches_reference() {
        let mut f = defaults();
        set(&mut f, "velocity", &[0.0]);
        set(&mut f, "air_resistance", &[0.0]);
        set(&mut f, "gravity", &[100.0]);
        set(&mut f, "rate", &[30.0]);
        let st = run(&f, 31);
        let q = st.ps.iter().find(|q| (q.age - 1.0).abs() < 1e-3).unwrap();
        assert!((q.p[1] - 540.0 - 100.0).abs() < 4.0, "{}", q.p[1]);
    }

    /// A drop of 550 px onto a floor at y = 850 (Gravity 500, Bounce 50%): ref B5 rises to
    /// 712.5 and is at 840.9 two frames after the bounce. With Bounce Random 30 the outgoing
    /// direction leans tan 0.37 off the normal and the speed is 0.55–0.85 of the plain bounce;
    /// particles resting on the floor never sink through it; Slide stays on it and stops.
    #[test]
    fn bounce() {
        let mut f = defaults();
        for (id, v) in [
            ("emitter_behavior", 1.0),
            ("rate", 1.0),
            ("velocity", 0.0),
            ("velocity_random", 0.0),
            ("velocity_from_motion", 0.0),
            ("air_resistance", 0.0),
            ("gravity", 500.0),
            ("life", 20.0),
            ("bounce", 1.0),
            ("bounce_random", 0.0),
        ] {
            set(&mut f, id, &[v]);
        }
        set(&mut f, "position", &[960.0, 300.0, 0.0]);
        set(&mut f, "floor_position", &[960.0, 850.0, 0.0]);
        let y = |frames| run(&f, frames).ps[0].p[1];
        assert!((y(33) - 840.89).abs() < 0.3, "{}", y(33));
        let apex = (34..60).map(y).fold(f32::INFINITY, f32::min);
        assert!((apex - 712.5).abs() < 1.0, "{apex}");
        assert!((y(600) - 850.0).abs() < 0.5, "{}", y(600));
        set(&mut f, "collision_event", &[1.0]);
        let st = run(&f, 120);
        assert_eq!(st.ps[0].p[1], 850.0);
        assert!(len3(st.ps[0].v) < 1e-3);
        set(&mut f, "collision_event", &[0.0]);
        set(&mut f, "bounce_random", &[30.0]);
        set(&mut f, "rate", &[400.0]);
        let st = run(&f, 34);
        let plain = 0.5 * (2.0 * 1000.0 * 550.0f32).sqrt();
        let (mut lean, mut lo, mut hi) = (0.0, f32::INFINITY, 0.0f32);
        for q in &st.ps {
            // the velocity at the bounce, 1.536 frames before
            let v = [q.v[0], q.v[1] - 1000.0 * (34.0 - 32.464) / 30.0, q.v[2]];
            lean += v[0].hypot(v[2]) / -v[1] / st.ps.len() as f32;
            let m = len3(v) / plain;
            (lo, hi) = (lo.min(m), hi.max(m));
        }
        assert!(
            (lean - 0.37).abs() < 0.03 && lo > 0.5 && hi < 0.9,
            "{lean} {lo} {hi}"
        );
    }

    /// A Floor Layer placed by EffectCraft's transform (an 800 x 400 layer at y 850 turned X
    /// Rotation 90, centred in the comp-size image grid): Layer Size catches a drop over its
    /// width only (ref BL1); a floor rising 100 px/s keeps a particle on top of it.
    #[test]
    fn floor_layer() {
        let mut f = defaults();
        for (id, v) in [
            ("emitter_behavior", 1.0),
            ("rate", 1.0),
            ("velocity", 0.0),
            ("velocity_random", 0.0),
            ("velocity_from_motion", 0.0),
            ("air_resistance", 0.0),
            ("gravity", 500.0),
            ("life", 20.0),
            ("bounce", 1.0),
            ("bounce_random", 0.0),
            ("floor_layer", 1.0),
            ("floor_mode", 1.0),
        ] {
            set(&mut f, id, &[v]);
        }
        // image (x, y, z) -> (x, y_floor - z, y - 540): its centre (960, 540) at (960, y_floor, 0)
        let xf = |y: f32| {
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, -1.0, y],
                [0.0, 1.0, 0.0, -540.0],
            ]
        };
        let rect = Some([560.0, 340.0, 1360.0, 740.0]);
        let sim = |x: f64, floor: &dyn Fn(u64) -> f32, frames: u64| {
            let mut g = f.clone();
            set(&mut g, "position", &[x, 300.0, 0.0]);
            let mut b = Params::from_flat(&g);
            let mut st = State::default();
            for k in 0..frames {
                b.floors[0].place(&xf(floor(k + 1)), &xf(floor(k)), rect);
                step(&mut st, &b, &b, 1.0 / 30.0);
            }
            st.ps[0].p[1]
        };
        let still = |_| 850.0;
        let apex = (34..60)
            .map(|k| sim(960.0, &still, k))
            .fold(f32::INFINITY, f32::min);
        assert!((apex - 712.5).abs() < 1.0, "{apex}");
        assert!(sim(1500.0, &still, 60) > 1200.0);
        let rising = |k: u64| 900.0 - 100.0 * (k as f32 / 30.0).min(2.0);
        for k in [33, 45, 60, 90, 120] {
            let y = sim(960.0, &rising, k);
            assert!(y <= rising(k) + 0.01, "frame {k}: {y} under the floor");
        }
    }

    /// A Star at R = 50 (ref ST2): solid core, rays 10 px wide at R, 5 px at 2R, gone at 3R;
    /// a Streaks particle's pattern: the centre dot first, the rest in the unit ball at 0.2–0.75 size.
    #[test]
    fn star_and_streaks() {
        let d = Dot {
            r: 50.0,
            w: 3.1 + 6.0,
            rot: [1.0, 0.0],
            ..Dot::default()
        };
        let width = |x: f32| -> f32 {
            (-20..=20)
                .map(|y| star_cov(&d, 0.0, x * x, x, y as f32, true)[0])
                .sum()
        };
        assert_eq!(star_cov(&d, 0.0, 0.0, 0.0, 0.0, true)[0], 1.0);
        assert!((width(50.0) - 10.0).abs() < 0.05, "{}", width(50.0));
        assert!((width(100.0) - 5.0).abs() < 0.05, "{}", width(100.0));
        assert_eq!(width(150.0), 0.0);
        let mut f = defaults();
        set(&mut f, "streak_count", &[30.0]);
        let pat = streak_pattern(&Params::from_flat(&f));
        assert_eq!((pat.len(), pat[0]), (30, [0.0, 0.0, 1.0]));
        assert!(
            pat[1..]
                .iter()
                .all(|p| p[0].hypot(p[1]) <= 1.0 && (0.2..=0.75).contains(&p[2]))
        );
    }

    /// Layer emitter: an explosion of 10 000 from a 100×50 layer whose left half is red at 50%
    /// alpha emits half of them, all on that half, red, at half opacity, 0–Size Z deep.
    #[test]
    fn layer_emitter() {
        let mut px = vec![[0.0f32; 4]; 100 * 50];
        for (i, p) in px.iter_mut().enumerate() {
            if i % 100 < 50 {
                *p = [0.5, 0.0, 0.0, 0.5];
            }
        }
        let mut opaque = vec![];
        // Text/Mask Edges: only the half's border, 50 + 50 + 2 x 48 pixels
        opaque_pixels(&px, 100, true, &mut opaque);
        assert_eq!(opaque.len(), 196);
        assert!(opaque.iter().all(|&i| {
            let (x, y) = (i % 100, i / 100);
            x == 0 || x == 49 || y == 0 || y == 49
        }));
        opaque_pixels(&px, 100, false, &mut opaque);
        let l = EmitLayer {
            px: &px,
            w: 100,
            h: 50,
            origin: [0.0; 2],
            scale: 1.0,
            opaque: &opaque,
            center: [50.0, 25.0],
            xf: None,
        };
        let mut f = defaults();
        for (id, v) in [
            ("emitter_behavior", 1.0),
            ("emitter_type", 3.0),
            ("emitter_layer", 1.0),
            ("rate", 10_000.0),
            ("velocity", 0.0),
            ("emitter_size_z", 200.0),
        ] {
            set(&mut f, id, &[v]);
        }
        set(&mut f, "position", &[50.0, 25.0, 0.0]);
        let p = Params::from_flat(&f);
        let mut st = State::default();
        step_with(&mut st, &p, &p, 1.0 / 30.0, Some(&l));
        assert_eq!(st.ps.len(), 5000);
        assert!(st.ps.iter().all(|q| q.p[0] < 50.0
            && (0.0..=200.0).contains(&q.p[2])
            && q.color == [1.0, 0.0, 0.0]
            && (q.opacity - 0.5).abs() < 1e-6));
    }

    /// On a Grid over a 100 x 50 layer, grey 0.25 on the left half and 0.75 on the right: a
    /// 4 x 2 grid at the cell centres; Lightness - Size halves and grows the size by 1.5, and
    /// Lightness - Velocity sends them back and forth; Periodic Burst at 16/s fires the
    /// 8 cells twice a second, Traverse one cell per event.
    #[test]
    fn layer_grid() {
        let px: Vec<Px> = (0..100 * 50)
            .map(|i| {
                if i % 100 < 50 {
                    [0.25, 0.25, 0.25, 1.0]
                } else {
                    [0.75, 0.75, 0.75, 1.0]
                }
            })
            .collect();
        let l = EmitLayer {
            px: &px,
            w: 100,
            h: 50,
            origin: [0.0; 2],
            scale: 1.0,
            opaque: &[],
            center: [50.0, 25.0],
            xf: None,
        };
        let mut f = defaults();
        for (id, v) in [
            ("emitter_behavior", 1.0),
            ("emitter_type", 3.0),
            ("emitter_layer", 1.0),
            ("on_a_grid", 1.0),
            ("grid_x", 4.0),
            ("grid_y", 2.0),
            ("grid_z", 1.0),
            ("rate", 1.0),
            ("velocity", 0.0),
            ("size", 10.0),
            ("size_random", 0.0),
            ("velocity_random", 0.0),
            ("layer_rgb", 2.0),
        ] {
            set(&mut f, id, &[v]);
        }
        set(&mut f, "position", &[50.0, 25.0, 0.0]);
        let run = |f: &[f64], frames: usize| {
            let p = Params::from_flat(f);
            let mut st = State::default();
            for _ in 0..frames {
                step_with(&mut st, &p, &p, 1.0 / 30.0, Some(&l));
            }
            st
        };
        let st = run(&f, 1);
        let mut at: Vec<(f32, f32, f32)> = st.ps.iter().map(|q| (q.p[0], q.p[1], q.size)).collect();
        at.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(at.len(), 8);
        for (k, &(x, y, s)) in at.iter().enumerate() {
            assert!((x - (12.5 + 25.0 * (k / 2) as f32)).abs() < 1e-3);
            assert!((y - (12.5 + 25.0 * (k % 2) as f32)).abs() < 1e-3);
            assert!((s - if x < 50.0 { 5.0 } else { 15.0 }).abs() < 1e-3, "{s}");
        }
        // Layer Scale 50%, about the layer's centre: the first cell (12.5, 12.5) at (31.25, 18.75)
        set(&mut f, "layer_scale", &[50.0]);
        let st = run(&f, 1);
        let first = st
            .ps
            .iter()
            .map(|q| (q.p[0], q.p[1]))
            .fold((f32::MAX, f32::MAX), |a, p| (a.0.min(p.0), a.1.min(p.1)));
        assert!(
            (first.0 - 31.25).abs() < 1e-3 && (first.1 - 18.75).abs() < 1e-3,
            "{first:?}"
        );
        set(&mut f, "layer_scale", &[100.0]);
        set(&mut f, "layer_rgb", &[3.0]);
        set(&mut f, "velocity", &[100.0]);
        set(&mut f, "air_resistance", &[0.0]); // the launch speed itself, not drag on its first step
        let st = run(&f, 1);
        assert!(st.ps.iter().all(|q| (len3(q.v) - 100.0).abs() < 1e-3));
        set(&mut f, "air_resistance", &[0.5]);
        set(&mut f, "emitter_behavior", &[0.0]);
        set(&mut f, "rate", &[16.0]);
        assert_eq!(run(&f, 30).ps.len(), 16);
        set(&mut f, "grid_sequencing", &[1.0]);
        assert_eq!(run(&f, 30).ps.len(), 16);
        // EffectCraft's own layer transform (here: 1000 px right, mirrored in depth) replaces
        // Position and the Layer Rotation / Scale controls
        let m = [
            [1.0, 0.0, 0.0, 1000.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, 0.0],
        ];
        let lx = EmitLayer {
            xf: Some([m, m]),
            ..l
        };
        set(&mut f, "emitter_behavior", &[1.0]);
        set(&mut f, "layer_scale", &[50.0]);
        set(&mut f, "velocity", &[0.0]);
        let p = Params::from_flat(&f);
        let mut st = State::default();
        step_with(&mut st, &p, &p, 1.0 / 30.0, Some(&lx));
        let xs: Vec<f32> = st.ps.iter().map(|q| q.p[0]).collect();
        assert!(
            xs.iter().all(|x| [1012.5, 1037.5, 1062.5, 1087.5]
                .iter()
                .any(|c| (x - c).abs() < 1e-3)),
            "{xs:?}"
        );
    }

    /// Same parameters, same frame: identical particles whether simulated in one go or resumed.
    #[test]
    fn deterministic() {
        let f = defaults();
        let a = run(&f, 40);
        let p = Params::from_flat(&f);
        let mut b = run(&f, 25);
        for _ in 0..15 {
            step(&mut b, &p, &p, 1.0 / 30.0);
        }
        assert_eq!(format!("{:?}", a.ps), format!("{:?}", b.ps));
    }

    /// A 100 px sphere at feather 50 is half transparent 74 px out (ref P2: 73.5).
    #[test]
    fn sphere_profile() {
        let (r, w) = (100.0, (0.5f32 * 100.0 + 3.25).min(100.5));
        assert!((sphere(73.5, r, w) - 0.5).abs() < 0.02);
        assert!(sphere(40.0, r, w) > 0.999 && sphere(101.0, r, w) == 0.0);
    }

    /// 8 orange glow spheres on one point; at the centre the sphere covers 1 and the glow 0.25.
    /// Add keeps adding light (no cap at the particle's own colour, so a dense core gets hotter);
    /// Normal with an Add glow is the last sphere plus its glow (ref G6).
    #[test]
    fn glow_composite() {
        let c = [1.0, 0.4, 0.1];
        let stack = |n: usize, f: &dyn Fn(&mut Px)| {
            let mut o = [0.0; 4];
            (0..n).for_each(|_| f(&mut o));
            o
        };
        let near = |a: Px, b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.006);
        // (with an Add glow too: a Normal glow is laid over what's beneath it, not added)
        let add = |o: &mut Px| put::<1>(o, c, 1.0, [1.0, 0.25], 1);
        let (one, eight) = (stack(1, &add), stack(8, &add));
        for ch in 0..3 {
            assert!(
                (eight[ch] - 8.0 * one[ch]).abs() < 1e-3 * eight[ch].max(1.0),
                "{one:?} {eight:?}"
            );
        }
        assert!(eight[0] > 1.0 && (eight[3] - 1.0).abs() < 1e-6, "{eight:?}");
        // two Add particles of half coverage: light adds, coverage unites
        let mut o = [0.0; 4];
        blend_pm::<1>(&mut o, [0.25, 0.1, 0.0], 0.5);
        blend_pm::<1>(&mut o, [0.25, 0.1, 0.0], 0.5);
        assert!(near(o, [0.5, 0.2, 0.0, 0.75]), "{o:?}");
        // Normal sphere then its Add glow on top: the last sphere's colour plus a quarter of it
        assert!(near(
            stack(8, &|o| put::<0>(o, c, 1.0, [1.0, 0.25], 1)),
            [1.25, 0.5, 0.125, 1.0]
        ));
    }
    /// The random-unit-gradient noise (with sin/cos per gradient) the turbulence was tuned with.
    fn old_noise3(p: [f32; 3]) -> f32 {
        let fl = p.map(f32::floor);
        let f = [p[0] - fl[0], p[1] - fl[1], p[2] - fl[2]];
        let i = fl.map(|x| x as i32);
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let grad = |x: i32, y: i32, z: i32, d: [f32; 3]| {
            let h = mix((x as u32 as u64) | ((y as u32 as u64) << 21) ^ ((z as u32 as u64) << 42));
            let g = unit_sphere(
                (h & 0xffff) as f32 / 65536.0,
                ((h >> 16) & 0xffff) as f32 / 65536.0,
            );
            g[0] * d[0] + g[1] * d[1] + g[2] * d[2]
        };
        let (u, v, w) = (fade(f[0]), fade(f[1]), fade(f[2]));
        let mut acc = [0.0f32; 8];
        for (n, a) in acc.iter_mut().enumerate() {
            let c = [(n & 1) as i32, ((n >> 1) & 1) as i32, ((n >> 2) & 1) as i32];
            *a = grad(
                i[0] + c[0],
                i[1] + c[1],
                i[2] + c[2],
                [f[0] - c[0] as f32, f[1] - c[1] as f32, f[2] - c[2] as f32],
            );
        }
        let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
        l(
            l(l(acc[0], acc[1], u), l(acc[2], acc[3], u), v),
            l(l(acc[4], acc[5], u), l(acc[6], acc[7], u), v),
            w,
        ) * 1.6
    }

    /// The fast noise has the spread the turbulence constants were fitted with (within 3%).
    #[test]
    fn noise_gain() {
        let (mut old, mut new) = (0.0f64, [0.0f64; 3]);
        let n = 200_000;
        for k in 0..n {
            let p = [
                rnd(1, k, 0) * 50.0,
                rnd(1, k, 1) * 50.0,
                rnd(1, k, 2) * 50.0,
            ];
            old += (old_noise3(p) as f64).powi(2);
            let v = noise3v(p);
            for a in 0..3 {
                new[a] += (v[a] as f64).powi(2);
            }
        }
        let so = (old / n as f64).sqrt();
        for a in new {
            let sn = (a / n as f64).sqrt();
            eprintln!("rms old {so:.4} new {sn:.4} ratio {:.4}", so / sn);
            assert!((sn / so - 1.0).abs() < 0.03, "noise rms {sn} vs {so}");
        }
    }
    /// One main particle at 500 px/s emitting 30 aux particles/s that stay put (velocity 0,
    /// life 1 s): a trail of 30, 16.7 px apart, ending where the main particle was 1 s ago; the
    /// same at 30 and 60 frames/s (births follow the particle's age, not the frames).
    #[test]
    fn aux_trail() {
        let mut f = defaults();
        set(&mut f, "emitter_behavior", &[1.0]);
        set(&mut f, "rate", &[1.0]);
        set(&mut f, "position", &[100.0, 540.0, 0.0]);
        set(&mut f, "direction", &[1.0]);
        set(&mut f, "direction_spread", &[0.0]);
        set(&mut f, "y_rotation", &[90.0]);
        set(&mut f, "velocity", &[500.0]);
        set(&mut f, "velocity_random", &[0.0]);
        set(&mut f, "air_resistance", &[0.0]);
        set(&mut f, "life", &[4.0]);
        set(&mut f, "aux_emit", &[1.0]);
        set(&mut f, "aux_rate", &[30.0]);
        set(&mut f, "aux_life", &[1.0]);
        let p = Params::from_flat(&f);
        let trail = |fps: f32, frames: u64| {
            let mut st = State::default();
            for _ in 0..frames {
                step(&mut st, &p, &p, 1.0 / fps);
            }
            let mut xs: Vec<f32> = st.aux.iter().map(|q| q.p[0]).collect();
            xs.sort_by(f32::total_cmp);
            (xs, st.ps[0].p[0])
        };
        let (a, head) = trail(30.0, 60);
        let (b, _) = trail(60.0, 120);
        assert!((29..=31).contains(&a.len()), "{} aux particles", a.len());
        for w in a.windows(2) {
            assert!(
                (w[1] - w[0] - 500.0 / 30.0).abs() < 0.01,
                "spacing {}",
                w[1] - w[0]
            );
        }
        assert!(
            a[0] >= head - 500.0 - 0.01 && *a.last().unwrap() <= head,
            "trail {}..{} behind {head}",
            a[0],
            a.last().unwrap()
        );
        assert_eq!(a.len(), b.len());
        assert!(
            a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.01),
            "30 vs 60 fps trails differ"
        );
    }
    /// Time Sampling against refs TS1-TS11 (2 s clip, life 5): the clip time a particle of a
    /// given age shows at comp time 3.
    #[test]
    fn sprite_time_sampling() {
        let mut f = defaults();
        set(&mut f, "clip_length", &[2.0]);
        let q = |age: f32| Particle {
            id: 7,
            p: [0.0; 3],
            v: [0.0; 3],
            age,
            life: 5.0,
            size: 1.0,
            opacity: 1.0,
            color: [1.0; 3],
            grad: 0.0,
            rot: 0.0,
            spin: 0.0,
        };
        let at = |mode: usize, age: f32| {
            let mut f = f.clone();
            set(&mut f, "time_sampling", &[mode as f64]);
            sprite_time(&q(age), &Params::from_flat(&f), 3.0)
        };
        let near = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-3);
        assert!(near(at(0, 1.0), 3.0)); // Current Time
        assert!(near(at(1, 1.5), 1.5) && at(1, 2.5).is_none()); // Start at Birth - Play Once
        assert!(near(at(2, 2.5), 0.5)); // Loop
        assert!(near(at(3, 2.5), 1.0)); // Stretch: age 2.5 of 5 -> half the clip
        assert!(near(at(10, 1.2), 1.8)); // Birth Time
        // Random: one offset per particle, Play Once wraps but ends after a clip's length
        let r = at(4, 0.0).unwrap();
        assert!((0.0..2.0).contains(&r));
        assert!(near(at(6, 1.5), (r + 1.5) % 2.0) && near(at(5, 1.5), (r + 1.5) % 2.0));
        assert!(at(5, 2.1).is_none());
        // Split Clip (2 clips of 1 s): the clip's start plus the age within it
        let c = at(7, 0.0).unwrap().round();
        assert!(c == 0.0 || c == 1.0);
        assert!(near(at(8, 1.25), c + 0.25) && at(7, 1.25).is_none());
        assert!(near(at(9, 2.5), c + 0.5));
    }

    /// A comp camera that is the default view (at the centre, zoom = width x 50 / 36 in front)
    /// draws what no camera does; one twice as far away draws the particles half as big.
    #[test]
    fn camera_projection() {
        let (w, h) = (400usize, 300usize);
        let mut f = defaults();
        set(&mut f, "rate", &[300.0]);
        set(&mut f, "position", &[200.0, 150.0, 0.0]);
        set(&mut f, "size", &[6.0]);
        let p = Params::from_flat(&f);
        let st = run(&f, 20);
        let draw = |cam: Option<&Camera>| {
            let mut px = vec![[0.0f32; 4]; w * h];
            let mut sc = Scratch::default();
            render(
                &mut px,
                w,
                h,
                &st,
                &p,
                1.0,
                [0.0; 2],
                [w as f32, h as f32],
                1.0 / 30.0,
                &mut sc,
                &Sprites::default(),
                None,
                cam,
                false,
            );
            px
        };
        let zoom = w as f64 * 50.0 / 36.0;
        let cam = |back: f64| Camera {
            m: [
                [zoom, 0.0, 200.0, 200.0 * back],
                [0.0, zoom, 150.0, 150.0 * back],
                [0.0, 0.0, 1.0, zoom + back],
            ],
            zoom,
            dof: None,
            shutter: None,
        };
        let (a, b) = (draw(None), draw(Some(&cam(0.0))));
        let d = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x[3] - y[3]).abs())
            .fold(0.0f32, f32::max);
        assert!(d < 1e-3, "{d}");
        // twice as far (the particles sit about z = 0): a quarter of the alpha, give or take
        let far = draw(Some(&cam(zoom)));
        let (sa, sf) = (
            a.iter().map(|p| p[3]).sum::<f32>(),
            far.iter().map(|p| p[3]).sum::<f32>(),
        );
        assert!((0.2..0.35).contains(&(sf / sa)), "{}", sf / sa);
    }

    /// A 40 x 20 sprite at Size 30 is drawn 60 x 30 (width = 2 x Size), and 30 x 60 at Rotate Z 90.
    #[test]
    fn sprite_geometry() {
        let (w, h) = (400usize, 300usize);
        let mut img = vec![[0.0f32; 4]; w * h];
        for y in 100..120 {
            for x in 50..90 {
                img[y * w + x] = [1.0; 4];
            }
        }
        let mut f = defaults();
        set(&mut f, "emitter_behavior", &[1.0]);
        set(&mut f, "rate", &[1.0]);
        set(&mut f, "velocity", &[0.0]);
        set(&mut f, "velocity_random", &[0.0]);
        set(&mut f, "position", &[200.0, 150.0, 0.0]);
        set(&mut f, "particle_type", &[3.0]);
        set(&mut f, "sprite_layer", &[1.0]);
        set(&mut f, "size", &[30.0]);
        let extent = |rot: f64| {
            let mut f = f.clone();
            set(&mut f, "rotate_z", &[rot]);
            let p = Params::from_flat(&f);
            let mut st = State::default();
            // two steps: a sprite isn't drawn at age 0 (the reference)
            step(&mut st, &p, &p, 1.0 / 30.0);
            step(&mut st, &p, &p, 1.0 / 30.0);
            let mut sc = Scratch::default();
            let mut sprites = Sprites::default();
            sprites.insert(0, &img, w, h);
            let mut px = vec![[0.0f32; 4]; w * h];
            render(
                &mut px,
                w,
                h,
                &st,
                &p,
                1.0,
                [0.0; 2],
                [w as f32, h as f32],
                1.0 / 30.0,
                &mut sc,
                &sprites,
                None,
                None,
                false,
            );
            let on: Vec<(usize, usize)> = (0..w * h)
                .filter(|&i| px[i][3] > 0.5)
                .map(|i| (i % w, i / w))
                .collect();
            let wd =
                on.iter().map(|p| p.0).max().unwrap() - on.iter().map(|p| p.0).min().unwrap() + 1;
            let ht =
                on.iter().map(|p| p.1).max().unwrap() - on.iter().map(|p| p.1).min().unwrap() + 1;
            (wd, ht)
        };
        let (a, b) = (extent(0.0), extent(90.0));
        assert!(a.0.abs_diff(60) <= 1 && a.1.abs_diff(30) <= 1, "{a:?}");
        assert!(b.0.abs_diff(30) <= 1 && b.1.abs_diff(60) <= 1, "{b:?}");
    }

    #[test]
    fn lighting() {
        // ref LI1, LI6, LI8: Diffuse 80% x (250 / d)^2 from a point light; Ambient 20% from an
        // ambient one; a spot gives nothing outside its cone
        let lt = Lighting {
            ambient: 0.2,
            diffuse: 0.8,
            nominal: Some(250.0),
        };
        let light = |kind, pos, dir| ShadeLight {
            kind,
            pos,
            dir,
            color: [1.0; 3],
            half: 25f32.to_radians(),
            feather: 0.5,
        };
        let p = shade(&lt, &[light(2, [0.0, 0.0, -400.0], [0.0; 3])], [0.0; 3]);
        assert!((p[0] - 0.3125).abs() < 1e-5);
        assert_eq!(
            shade(&lt, &[light(3, [0.0; 3], [0.0; 3])], [9.0; 3]),
            [0.2; 3]
        );
        let spot = light(1, [0.0, 0.0, -400.0], [0.0, 0.0, 1.0]);
        assert!((shade(&lt, &[spot], [0.0; 3])[0] - 0.3125).abs() < 1e-5);
        assert_eq!(shade(&lt, &[spot], [400.0, 0.0, 0.0]), [0.0; 3]);
        let none = Lighting {
            nominal: None,
            ..lt
        };
        assert_eq!(
            shade(&none, &[light(2, [0.0, 0.0, -4000.0], [0.0; 3])], [0.0; 3]),
            [0.8; 3]
        );
    }

    #[test]
    fn between_frames() {
        let mut st = State::default();
        let mut f = defaults();
        set(&mut f, "velocity", &[300.0]);
        let p = Params::from_flat(&f);
        for _ in 0..10 {
            step(&mut st, &p, &p, 1.0 / 30.0);
        }
        let half = between(&st, 0.5 / 30.0);
        let (a, b) = (&st.ps[0], &half.ps[0]);
        assert_eq!(half.ps.len(), st.ps.len());
        assert!((b.p[0] - a.p[0] - a.v[0] * 0.5 / 30.0).abs() < 1e-4);
        assert!((b.age - a.age - 0.5 / 30.0).abs() < 1e-6);
    }

    #[test]
    fn obj_model() {
        // a unit cube's 8 corners and 6 quads (12 triangles, 12 edges), 2 units wide
        let mut t = String::new();
        for i in 0..8 {
            t += &format!(
                "v {} {} {}
",
                (i & 1) * 2,
                (i >> 1 & 1) * 2,
                (i >> 2) * 2
            );
        }
        for f in [
            "1 2 4 3", "5 6 8 7", "1 2 6 5", "3 4 8 7", "1 3 7 5", "2 4 8 6",
        ] {
            t += &format!(
                "f {f}
"
            );
        }
        let m = Mesh::from_obj(&t).unwrap();
        assert_eq!((m.v.len(), m.tri.len(), m.edges.len()), (8, 12, 12));
        // fitted into the unit cube; faces: on its surface
        for k in 0..100 {
            let u = |c| rnd(7, k, c);
            let p = m.sample(2, u(0), u(1), u(2));
            assert!(p.iter().all(|c| c.abs() <= 0.5 + 1e-5));
            assert!(p.iter().any(|c| (c.abs() - 0.5).abs() < 1e-4));
            let v = m.sample(0, u(0), u(1), u(2));
            assert!(v.iter().all(|c| (c.abs() - 0.5).abs() < 1e-5));
        }
    }
}
