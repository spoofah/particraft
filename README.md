# PartiCraft

A 3D particle system for [EffectCraft](https://github.com/storytold/effectcraft): emitters, physics,
sprites, lighting and motion blur, as one effect. It runs as a native **effect helper**, a separate
program that EffectCraft starts and renders through. It uses every core and draws tens of thousands
of particles per frame at 1080p in a few milliseconds.

The simulation is deterministic: a frame renders the same whether you scrub, play or render, and
whatever order the frames come in.

## Features

- **Emitters:** Point, Box, Sphere, Layer (emits from a layer's pixels, takes their colour, and can
  turn their colour into size, velocity or rotation), Light(s), Text/Mask outlines or fills, and 3D
  Model (an OBJ file's vertices, edges or faces).
  - Continuous or Explode, with five direction modes.
  - Distribution on a grid, with four sequencing types.
  - Position Subframe, so a fast-moving emitter leaves a smooth trail.
- **Particle types:**
  - Sphere and Glow Sphere, with their own blend modes and Glow Blend Mode.
  - Star and Streaklet.
  - Cloudlet (textured smoke).
  - Sprite: any layer, with rotation, Colorize / Fill and 11 time-sampling modes for animated sprites.
- **Look:**
  - Size, opacity and colour over life.
  - Colour random.
  - Normal, Add, Screen and Lighten blend modes.
- **Physics:**
  - Gravity, air resistance and wind.
  - Air turbulence (a fractal wind field).
  - **Bounce** off a floor layer and two wall layers: Bounce, Slide, Stick or Kill, with friction,
    and particles come to rest.
- **Aux system:** trails of secondary particles from each main particle, with their own life,
  size, opacity and colour over life.
- **Comp lights and camera:**
  - Lighting by the comp's point and spot lights.
  - Shadowlets (soft self-shadowing for clouds).
  - Drawing through the comp's camera layer, with the camera's depth of field.
- **Motion blur:** for particles, sprites and a moving camera.

148 controls, in 19 twirl-down groups: Emitter (with 3D Model, Layer Emitter, Distribution),
Particle (Sprite, Rotation, Glow, Streaklet), Physics (Air Turbulence, Bounce 1–3), Aux System,
Lighting, Shadowlets and Rendering (Motion Blur).

## Install

PartiCraft is a native program, so it isn't in EffectCraft's Plug-in Library, which carries
WebAssembly plug-ins only.

1. Download `effectcraft-helper-particraft.exe` from the
   [latest release](https://github.com/spoofah/particraft/releases/latest) and check it against
   `SHA256SUMS`.
2. Close EffectCraft and copy the exe into the `helpers` folder beside `effectcraft.exe`.
3. Start EffectCraft and choose **Effect ▸ Scan for Effect Helpers**. To start helpers every time
   EffectCraft launches, set the environment variable `EFFECTCRAFT_HELPERS=1`.
4. Apply **Simulation ▸ PartiCraft** to a solid.

Releases are Windows x64 builds. On Linux or macOS, build it from source (below); those builds
haven't been tested yet.

## Which EffectCraft

PartiCraft works with EffectCraft 0.24.1. Some features need host features that are on their way
into EffectCraft; until your EffectCraft has them, PartiCraft falls back as below.

| Needs | Without it |
|---|---|
| A render cache keyed on time for helper effects ([#3786](https://github.com/storytold/effectcraft/pull/3786)) | On a layer with nothing animated, the particles freeze on their first frame. Work-around: add the expression `time*0.001` to Physics ▸ Air Turbulence ▸ Evolution Offset. |
| Grouped controls ([#3751](https://github.com/storytold/effectcraft/pull/3751)) | The controls show as one flat list. Projects keep their values either way. |
| Layer transforms, comp camera and comp lights ([#3788](https://github.com/storytold/effectcraft/pull/3788)) | No drawing through a camera layer or depth of field; no Lighting, Light(s) emitter or Shadowlets light; Floor Position / Rotation / Width / Height stand in for a Floor Layer; Bounce 2 and 3 do nothing; a Layer emitter sits at Position and uses the Layer Rotation / Scale controls. |
| Gradient parameters ([#3789](https://github.com/storytold/effectcraft/pull/3789)) | Color over Life blends from Color to Gradient End Color instead of following a gradient. |
| Bounded generator renders and several helper processes ([#3790](https://github.com/storytold/effectcraft/pull/3790)) | Everything renders the same, just slower in projects with many PartiCraft layers. |

## Build

```bash
cargo test --release
cargo build --release --locked
```

The helper is `target/release/effectcraft-helper-particraft(.exe)`. The toolchain version is in
`rust-toolchain.toml`. Releases are built from the tag by `.github/workflows/release.yml`, so each
asset can be checked against the source.

`cargo run --release --example prof` times the simulation and drawing for each particle type in
process.

## How it works

EffectCraft starts the helper with `--effectcraft-helper` and talks to it over stdin and stdout
(the [effect helper protocol](https://github.com/storytold/effectcraft/blob/main/docs/external-helpers.md)).
Frames pass through a shared exchange file, which the helper maps into memory.

Each layer's particle state is kept between frames. A frame is simulated forward from the nearest
state before it: the 16 most recent states per layer are kept, because EffectCraft renders frames
out of order across its workers. Parameter changes behind the playhead are caught by sampling the
parameter history, so an edit re-simulates from where it takes effect.

Drawing is split into bands of rows over all cores. Generator layers report the rectangle they drew, so
EffectCraft composites only that.

## Licence

MIT or Apache-2.0, at your option.
