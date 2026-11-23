//! Particles as an EffectCraft external effect helper (docs/external-helpers.md, protocol 1).
//! EffectCraft starts it as `effectcraft-helper-particraft.exe --effectcraft-helper`.
//!
//! A frame at time t is the simulation from layer time 0 to t, one step per frame. Each applied
//! effect (`instance`) keeps its state, checkpoints every second, and the parameters it was
//! simulated with; every render re-reads the parameters of all earlier frames (`params_at`) and
//! resumes from the last checkpoint before the first frame whose parameters changed.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, Write};

use particraft::{
    Camera, EmitLayer, EmitLight, FloorMask, Grad, ID, Params, Px, Scratch, ShadeLight, Sprites,
    State, layer_colors, manifest, nflat, opaque_pixels, render, sprite_frames, step, step_with,
    to_layer_units,
};
use serde_json::{Value, json};

const CHECKPOINT: u64 = 30;
/// states kept besides the current one: an instance's frames arrive shuffled across ~16 frames
/// (EC renders several at once)
const PLAYHEADS: usize = 16;
/// Bounce Layer Alpha: renders a fetched floor layer mask serves before it's fetched again.
const FLOOR_REFETCH: usize = 8;
/// Parameter history probes per render (rolling) and the stride checked on a jump.
// ponytail: an edit to earlier frames shows up within known/ROLL renders of playback, not at once.
const ROLL: usize = 16;
const SPARSE: usize = 10;
/// Sprite frames fetched per render at most (EffectCraft gives each a new 33 MB exchange slot at
/// 1080p), and held per instance at most.
// ponytail: past SPRITE_HOLD frames particles show the nearest held one; an LRU if clips get long.
const SPRITE_FETCH: usize = 8;
const SPRITE_HOLD: usize = 300;

/// Parameters equal up to rounding (points come back through the preview scale).
fn same(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| (x - y).abs() <= 1e-7 * x.abs().max(1.0))
}

#[derive(Default)]
struct Instance {
    fps: f64,
    /// flat parameters (layer units) of every frame simulated so far
    hist: Vec<Vec<f64>>,
    /// Position Subframe: the emitter's Position inside the step ending at each frame (empty:
    /// not asked yet); dropped with `hist`
    // ponytail: a key edit that changes only in-between values (not at any frame) isn't noticed.
    sub: Vec<Vec<[f32; 3]>>,
    /// states at frames 0, CHECKPOINT, 2·CHECKPOINT…
    checkpoints: Vec<State>,
    cur: State,
    /// other playheads' states, least recently used first: EC renders several frames at once,
    /// so an instance's frames arrive out of order (and the viewer and a preview interleave)
    alt: Vec<State>,
    /// the frame rendered last, and where the rolling parameter check continues
    last: Option<u64>,
    sweep: usize,
    /// EffectCraft's digest of the instance's parameter data at the last render (`params_digest`:
    /// newer EffectCraft, absent while a parameter has an expression)
    digest: Option<u64>,
    /// Layer Sampling Still: the emitter layer at time 0, kept until a parameter changes
    still: Option<StillLayer>,
    /// the sprite layer's frames in use, the digest of each, the layer and frame grid they came
    /// from, and which one is re-checked next
    sprites: Sprites,
    sprite_sums: HashMap<i64, u64>,
    sprite_key: Option<(i64, usize, usize, u64, u64, u64)>,
    sprite_turn: usize,
    /// Layer Sampling Current Time: each particle's colour in the layer now (reused buffer)
    now_colors: Vec<[f32; 3]>,
    /// 3D Model: the OBJ File it was loaded from, and the mesh (None: no file or unreadable)
    model: Option<(String, Option<std::sync::Arc<particraft::Mesh>>)>,
    /// Bounce Layer Alpha, per bounce layer: its mask, the layer and frame grid it came from,
    /// and the renders it has served since it was fetched
    #[allow(clippy::type_complexity)]
    floor_mask: [Option<(
        (i64, usize, usize, u64, u64, u64),
        usize,
        std::sync::Arc<FloorMask>,
    )>; 3],
}

/// The emitter layer kept for Layer Sampling Still, and the frame grid it was fetched in.
struct StillLayer {
    px: Vec<Px>,
    w: usize,
    h: usize,
    opaque: Vec<u32>,
    scale: f64,
    origin: [f64; 2],
    edges: bool,
}

fn send(out: &mut impl Write, v: Value) {
    let _ = writeln!(out, "@ec-helper {v}").and_then(|()| out.flush());
}

/// A JSON array of three numbers.
fn f3(x: &Value) -> Option<[f32; 3]> {
    let a = x.as_array()?;
    Some([
        a.first()?.as_f64()? as f32,
        a.get(1)?.as_f64()? as f32,
        a.get(2)?.as_f64()? as f32,
    ])
}

struct Io<'a, I: Iterator<Item = std::io::Result<String>>> {
    lines: I,
    out: std::io::StdoutLock<'a>,
    /// EffectCraft answers `layer_transform` / `comp_camera` (it listed them in `hello`)
    xf: bool,
    cam: bool,
    /// ... and `comp_camera` at a time (`comp_camera_time`), and `comp_lights`
    cam_t: bool,
    lights: bool,
    /// ... and takes gradient parameters (`gradient_params`: Color over Life)
    gradients: bool,
    /// ... and honours `ignores_input` and the `bounds` of `done` (`frame_bounds`): only the
    /// rows the particles cover are drawn and read back
    bounds: bool,
}

impl<I: Iterator<Item = std::io::Result<String>>> Io<'_, I> {
    /// Sends `req` and waits for the reply with op `want`.
    fn call(&mut self, req: Value, want: &str) -> Result<Value, String> {
        send(&mut self.out, req);
        loop {
            let line = self
                .lines
                .next()
                .ok_or("EffectCraft closed the pipe")?
                .map_err(|e| e.to_string())?;
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match v["op"].as_str() {
                Some(op) if op == want => return Ok(v),
                Some("ping") => send(&mut self.out, json!({"op": "pong"})),
                // ponytail: no nested renders: parameters need none, and a sprite layer that
                // itself uses Particles would (it fails with this message).
                Some(op) => send(
                    &mut self.out,
                    json!({"op": "error", "message": format!("`{op}` while waiting for `{want}`")}),
                ),
                None => {}
            }
        }
    }

    /// The comp's camera layer as this layer sees it; None without one (or when EffectCraft
    /// doesn't say: then the default view, the reference's with no camera layer).
    fn comp_camera(&mut self, time: Option<f64>) -> Result<Option<Camera>, String> {
        if !self.cam || (time.is_some() && !self.cam_t) {
            return Ok(None);
        }
        let req = match time {
            Some(t) => json!({"op": "comp_camera", "time": t}),
            None => json!({"op": "comp_camera"}),
        };
        let v = self.call(req, "camera")?;
        let c = &v["camera"];
        if c.is_null() || c["camera_layer"].as_bool() != Some(true) {
            return Ok(None);
        }
        let mut m = [[0.0f64; 4]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, x) in row.iter_mut().enumerate() {
                *x = c["matrix"][i][j].as_f64().ok_or("bad camera")?;
            }
        }
        let zoom = c["zoom"]
            .as_f64()
            .filter(|z| *z > 0.0)
            .ok_or("bad camera zoom")?;
        // [focus, aperture, blur level] while its Depth of Field is on (newer EffectCraft)
        let dof = c["dof"].as_array().and_then(|d| {
            let v: Vec<f64> = d.iter().filter_map(|x| x.as_f64()).collect();
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        });
        Ok(Some(Camera {
            m,
            zoom,
            dof,
            shutter: None,
        }))
    }

    /// The comp's lights at layer time `t` as EffectCraft describes them; empty when it doesn't
    /// hand lights over.
    fn comp_lights_raw(&mut self, t: f64) -> Result<Vec<Value>, String> {
        if !self.lights {
            return Ok(Vec::new());
        }
        let mut v = self.call(json!({"op": "comp_lights", "time": t}), "lights")?;
        Ok(match v["lights"].take() {
            Value::Array(ls) => ls,
            _ => Vec::new(),
        })
    }

    /// The lights the Light(s) emitter emits from at layer time `t`: those named "Emitter..."
    /// (the reference's Light Naming default), ambient ones aside.
    fn comp_lights(&mut self, t: f64) -> Result<Vec<EmitLight>, String> {
        Ok(self
            .comp_lights_raw(t)?
            .iter()
            .filter(|l| l["name"].as_str().is_some_and(|n| n.starts_with("Emitter")))
            .filter(|l| l["kind"].as_u64().is_some_and(|k| k < 3))
            .filter_map(|l| {
                Some(EmitLight {
                    pos: f3(&l["pos"])?,
                    intensity: l["intensity"].as_f64()? as f32,
                })
            })
            .collect())
    }

    /// Every light at layer time `t`, for Lighting (ref LS14: "Emitter..." lights light too).
    fn shade_lights(&mut self, t: f64) -> Result<Vec<ShadeLight>, String> {
        self.lights_named(t, "")
    }

    /// The lights at layer time `t` whose name starts with `prefix`.
    fn lights_named(&mut self, t: f64, prefix: &str) -> Result<Vec<ShadeLight>, String> {
        Ok(self
            .comp_lights_raw(t)?
            .iter()
            .filter(|l| l["name"].as_str().is_some_and(|n| n.starts_with(prefix)))
            .filter_map(|l| {
                let i = l["intensity"].as_f64()? as f32;
                let c = f3(&l["color"])?;
                Some(ShadeLight {
                    kind: l["kind"].as_u64()?.min(3) as u8,
                    pos: f3(&l["pos"])?,
                    dir: f3(&l["dir"])?,
                    color: c.map(|c| c * i),
                    half: (l["cone"].as_f64().unwrap_or(90.0) as f32 / 2.0).to_radians(),
                    // hosts that don't say: 50%, the reference's default
                    feather: l["feather"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0) as f32,
                })
            })
            .collect())
    }

    /// Where the layer of layer parameter `param` sits at layer time `t` (image x, y and depth
    /// → this layer's pixels and depth), and its own extent in that image (x0, y0, x1, y1; None
    /// from EffectCraft builds before it was handed over); None when EffectCraft doesn't say.
    #[allow(clippy::type_complexity)]
    fn layer_transform(
        &mut self,
        param: &str,
        t: f64,
    ) -> Result<Option<([[f32; 4]; 3], Option<[f32; 4]>)>, String> {
        if !self.xf {
            return Ok(None);
        }
        let v = self.call(
            json!({"op": "layer_transform", "param": param, "time": t}),
            "transform",
        )?;
        let m = &v["transform"]["matrix"];
        if m.is_null() {
            return Ok(None);
        }
        let mut o = [[0.0f32; 4]; 3];
        for (i, row) in o.iter_mut().enumerate() {
            for (j, x) in row.iter_mut().enumerate() {
                *x = m[i][j].as_f64().ok_or("bad layer transform")? as f32;
            }
        }
        let r = &v["transform"]["rect"];
        let rect = (0..4)
            .map(|i| r[i].as_f64().map(|x| x as f32))
            .collect::<Option<Vec<f32>>>()
            .filter(|r| r[2] > r[0] && r[3] > r[1])
            .map(|r| [r[0], r[1], r[2], r[3]]);
        Ok(Some((o, rect)))
    }

    /// Asks EffectCraft for the parameters at layer time `t`.
    fn params_at(&mut self, t: f64) -> Result<Vec<f64>, String> {
        let v = self.call(json!({"op": "params_at", "time": t}), "params")?;
        Ok(flat(&v["values"][0]))
    }

    /// The layer of layer parameter `param` at layer time `t`, as (offset, width, height) in the
    /// exchange file; None when it has none.
    fn layer_input(&mut self, param: &str, t: f64) -> Result<Option<(u64, usize, usize)>, String> {
        let v = self.call(
            json!({"op": "layer_input", "param": param, "time": t}),
            "image",
        )?;
        let i = &v["image"];
        if i.is_null() {
            return Ok(None);
        }
        let num = |k: &str| i[k].as_f64().ok_or_else(|| format!("bad image ref {i}"));
        Ok(Some((
            num("offset")? as u64,
            num("width")? as usize,
            num("height")? as usize,
        )))
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() != Some("--effectcraft-helper") {
        eprintln!(
            "This is an EffectCraft effect helper: put it in EffectCraft's helpers folder and use Effect > Scan for Effect Helpers."
        );
        std::process::exit(2);
    }
    let mut frames: Option<File> = None;
    let mut view: Option<memmap2::MmapMut> = None;
    let mut insts: HashMap<i64, Instance> = HashMap::new();
    let mut scratch = Scratch::default();
    let mut opaque = Vec::new();
    let stdin = std::io::stdin().lock();
    let mut io = Io {
        lines: stdin.lines(),
        out: std::io::stdout().lock(),
        xf: false,
        cam: false,
        cam_t: false,
        lights: false,
        gradients: false,
        bounds: false,
    };
    while let Some(Ok(line)) = io.lines.next() {
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            eprintln!("unreadable request: {line:.200}");
            continue;
        };
        match req["op"].as_str().unwrap_or("") {
            "hello" => {
                view = None;
                let has = |name: &str| {
                    req["features"]
                        .as_array()
                        .is_some_and(|f| f.iter().any(|x| x == name))
                };
                io.xf = has("layer_transform");
                io.cam = has("comp_camera");
                io.cam_t = has("comp_camera_time");
                io.lights = has("comp_lights");
                io.gradients = has("gradient_params");
                io.bounds = has("frame_bounds");
                frames = req["frames"].as_str().and_then(|p| {
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(p)
                        .map_err(|e| eprintln!("frame exchange {p}: {e}"))
                        .ok()
                });
                // `processes`: EffectCraft may run several of these at once (instances spread over
                // them). An instance's simulation here is only a cache: the same parameters give
                // the same frames in any process.
                send(
                    &mut io.out,
                    json!({"op": "hello", "version": 1, "name": "PartiCraft", "helper_version": env!("CARGO_PKG_VERSION"), "loads": [], "processes": 16}),
                );
            }
            // time_varying: a simulation differs every frame even when nothing is animated
            // (EC hosts before it froze such layers; they ignore the field).
            "describe" => send(
                &mut io.out,
                json!({"op": "effects", "effects": [{"manifest": manifest(io.gradients), "padding": false, "time_varying": true, "ignores_input": io.bounds}], "errors": []}),
            ),
            "render" => {
                let r = render_req(
                    &req,
                    frames.as_ref(),
                    &mut view,
                    &mut insts,
                    &mut io,
                    &mut scratch,
                    &mut opaque,
                );
                match r {
                    Ok(None) => send(&mut io.out, json!({"op": "done"})),
                    Ok(Some(b)) => send(&mut io.out, json!({"op": "done", "bounds": b})),
                    Err(e) => send(&mut io.out, json!({"op": "error", "message": e})),
                }
            }
            "padding" => send(&mut io.out, json!({"op": "padding", "pixels": 0})),
            "load" => send(
                &mut io.out,
                json!({"op": "error", "message": "Particles loads no files"}),
            ),
            "ping" => send(&mut io.out, json!({"op": "pong"})),
            "destroy" => {
                insts.remove(&req["instance"].as_i64().unwrap_or(-1));
            }
            "shutdown" => break,
            op => send(
                &mut io.out,
                json!({"op": "error", "message": format!("unknown request `{op}`")}),
            ),
        }
    }
}

fn render_req<I: Iterator<Item = std::io::Result<String>>>(
    req: &Value,
    frames: Option<&File>,
    view: &mut Option<memmap2::MmapMut>,
    insts: &mut HashMap<i64, Instance>,
    io: &mut Io<'_, I>,
    scratch: &mut Scratch,
    opaque: &mut Vec<u32>,
) -> Result<Option<[u32; 4]>, String> {
    if req["effect"].as_str() != Some(ID) {
        return Err(format!("unknown effect {}", req["effect"]));
    }
    let f = &req["frame"];
    let num = |v: &Value| v.as_f64().ok_or_else(|| format!("bad frame ref {f}"));
    let (offset, w, h) = (
        num(&f["offset"])? as u64,
        num(&f["width"])? as usize,
        num(&f["height"])? as usize,
    );
    let scale = f["scale"].as_f64().unwrap_or(1.0);
    let origin = [
        f["origin"][0].as_f64().unwrap_or(0.0),
        f["origin"][1].as_f64().unwrap_or(0.0),
    ];
    let layer = [
        req["layer_size"][0].as_f64().unwrap_or(w as f64 / scale),
        req["layer_size"][1].as_f64().unwrap_or(h as f64 / scale),
    ];
    let fps = req["frame_rate"]
        .as_f64()
        .filter(|&r| r > 0.0)
        .unwrap_or(30.0);
    let time = req["time"].as_f64().unwrap_or(0.0);
    let n = nflat();
    let mut now = flat(&req["flat"]);
    if now.len() < n {
        return Err(format!("expected {n} parameters, got {}", now.len()));
    }
    to_layer_units(&mut now, scale, origin);

    // Simulate up to the frame at or before `time`.
    let target = (time * fps + 1e-6).floor().max(0.0) as u64;
    let inst = insts
        .entry(req["instance"].as_i64().unwrap_or(0))
        .or_default();
    // OBJ File (the first text parameter): a new file starts the simulation over
    let path = req["texts"][0]
        .as_str()
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .to_string();
    if inst.fps != fps || inst.model.as_ref().is_some_and(|m| m.0 != path) {
        *inst = Instance {
            fps,
            ..Default::default()
        };
    }
    if inst.model.is_none() {
        let mesh = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| particraft::Mesh::from_obj(&t))
            .map(std::sync::Arc::new);
        inst.model = Some((path, mesh));
    }
    let t0 = std::time::Instant::now();
    now.truncate(n);
    let mut fetch = |k: usize| -> Result<Vec<f64>, String> {
        let t = k as f64 / fps;
        if (t - time).abs() < 1e-9 {
            return Ok(now.clone());
        }
        let mut p = io.params_at(t)?;
        if p.len() < n {
            return Err(format!("params_at({t}) gave {} values", p.len()));
        }
        to_layer_units(&mut p, scale, origin);
        p.truncate(n);
        Ok(p)
    };
    // Find the first frame whose parameters differ from what was simulated (`hist`). Asking EC
    // costs ~0.12 ms a frame (220 ms per render if all 1800 frames of a 60 s layer were asked),
    // and EC won't re-render the frame shown after an edit at another time, so every render
    // probes a little: the frame before, a rolling slice of ROLL frames (the whole history every
    // few seconds of playback), and on a jump every SPARSE-th frame. Any mismatch, the same frame
    // asked again, or a change at the frame itself checks every frame for the first change.
    // With EffectCraft's parameter digest none of that is needed: the same digest as last time
    // means no parameter changed at any time (no expressions), and a new one means an edit,
    // found by checking every frame at once.
    let t_ = target as usize;
    let known = inst.hist.len().min(t_ + 1);
    let pd = req["params_digest"].as_u64();
    let unchanged = pd.is_some() && pd == inst.digest;
    let edited = pd.is_some() && inst.digest.is_some() && !unchanged;
    inst.digest = pd;
    let mut suspect = !unchanged
        && (edited
            || inst.last == Some(target)
            || (t_ < inst.hist.len() && !same(&inst.hist[t_], &fetch(t_)?)));
    if !suspect && !unchanged && known > 0 {
        let mut probe: Vec<usize> = (0..ROLL).map(|i| (inst.sweep + i) % known).collect();
        inst.sweep = (inst.sweep + ROLL) % known;
        probe.push(known - 1);
        // a jump; EC's render lanes deliver playback a few frames out of order, which isn't one
        if inst.last.is_none_or(|l| target.abs_diff(l) > 8) {
            probe.extend((0..known).step_by(SPARSE));
        }
        for k in probe {
            if !same(&inst.hist[k], &fetch(k)?) {
                suspect = true;
                break;
            }
        }
    }
    // no change found: everything simulated so far stays (also beyond the target: another
    // playhead may be there)
    let mut changed = inst.hist.len();
    if suspect {
        for (k, h) in inst.hist[..known].iter().enumerate() {
            let f = fetch(k)?;
            if !same(h, &f) {
                if let Some(log) = std::env::var_os("PARTICLES_LOG") {
                    let diff: Vec<String> = h
                        .iter()
                        .zip(&f)
                        .enumerate()
                        .filter(|(_, (a, b))| a != b)
                        .map(|(i, (a, b))| format!("#{i} {a} -> {b}"))
                        .take(6)
                        .collect();
                    let _ = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(log)
                        .and_then(|mut fl| {
                            writeln!(
                                fl,
                                "  render of frame {target} (scale {scale}): frame {k} changed: {}",
                                diff.join(", ")
                            )
                        });
                }
                changed = k;
                break;
            }
        }
    }
    if changed < inst.hist.len() {
        inst.still = None;
    }
    inst.hist.truncate(changed);
    inst.sub.truncate(changed);
    for k in inst.hist.len()..=t_ {
        let p = fetch(k)?;
        inst.hist.push(p);
    }
    inst.last = Some(target);
    let params = &inst.hist;
    let changed = changed as u64;
    // State at frame k depends on the parameters of frames 0..=k.
    inst.checkpoints.retain(|s| s.k < changed);
    // Resume from the latest valid state at or before the target: the current one, another
    // playhead's, or a checkpoint. The states not resumed from stay as playheads.
    let ok = |s: &State| s.k == 0 || s.k < changed;
    if !ok(&inst.cur) {
        inst.cur = State::default();
    }
    inst.alt.retain(|s| s.k > 0 && ok(s));
    let usable = |s: &State| (s.k <= target).then_some(s.k);
    let best_alt = (0..inst.alt.len()).max_by_key(|&i| usable(&inst.alt[i]));
    let (c, a) = (
        usable(&inst.cur),
        best_alt.and_then(|i| usable(&inst.alt[i])),
    );
    let p = inst
        .checkpoints
        .iter()
        .rev()
        .find(|s| s.k <= target)
        .map(|s| s.k);
    // The state resumed from is kept too (a copy costs far less than the steps another frame
    // behind this one would otherwise re-simulate).
    let keep = |alt: &mut Vec<State>, s: State| {
        if s.k > 0 && !alt.iter().any(|o| o.k == s.k) {
            if alt.len() >= PLAYHEADS {
                alt.remove(0);
            }
            alt.push(s);
        }
    };
    if a > c && a >= p {
        let i = best_alt.unwrap();
        let start = inst.alt.remove(i);
        if start.k < target {
            keep(&mut inst.alt, start.clone());
        }
        let old = std::mem::replace(&mut inst.cur, start);
        keep(&mut inst.alt, old);
    } else if c.is_none() || p > c.max(a) {
        let start = inst
            .checkpoints
            .iter()
            .rev()
            .find(|s| s.k <= target)
            .cloned()
            .unwrap_or_default();
        let old = std::mem::replace(&mut inst.cur, start);
        keep(&mut inst.alt, old);
    } else if inst.cur.k < target {
        keep(&mut inst.alt, inst.cur.clone());
    }
    let t1 = std::time::Instant::now();
    let from = inst.cur.k as usize;
    // only the frames stepped through (and the one drawn)
    let lo = from.min(target as usize);
    let mut ps: Vec<Params> = params[lo..=target as usize]
        .iter()
        .map(|p| Params::from_flat(p))
        .collect();
    // 3D Model: the mesh for every step
    let mesh = inst.model.as_ref().and_then(|m| m.1.clone());
    for b in ps.iter_mut() {
        b.model = mesh.clone();
    }
    // Color over Life (hosts with gradient parameters), for every step: the aux's Color From
    // Main reads the main colour at its birth
    // ponytail: an animated gradient is read at the frame drawn, also for births in a jump.
    let ids = particraft::text_ids(io.gradients);
    let grad = |id: &str| {
        let i = ids.iter().position(|t| *t == id)?;
        req["texts"][i]
            .as_str()
            .and_then(Grad::from_json)
            .map(std::sync::Arc::new)
    };
    let (g, ag) = (grad("color_over_life"), grad("aux_color_over_life"));
    for b in ps.iter_mut() {
        b.gradient = g.clone();
        b.aux_gradient = ag.clone();
    }
    // Position Subframe: a moving emitter's Position at 9 times inside each step, asked once
    // per frame (~1 ms of params_at) and kept until the parameters change
    // (from the first frame on: the step after it reads its samples for its start tangent)
    for i in 0..ps.len() {
        let k = lo + i;
        let prev = match i {
            0 if k == 0 => continue,
            0 => Params::from_flat(&inst.hist[k - 1]).pos,
            _ => ps[i - 1].pos,
        };
        if ps[i].subframe == 0 || ps[i].pos == prev {
            continue;
        }
        if inst.sub.len() <= k {
            inst.sub.resize(k + 1, Vec::new());
        }
        if inst.sub[k].is_empty() {
            for j in 1..10 {
                let mut p = io.params_at((k as f64 - 1.0 + j as f64 / 10.0) / fps)?;
                if p.len() < n {
                    return Err(format!("params_at gave {} values", p.len()));
                }
                to_layer_units(&mut p, scale, origin);
                inst.sub[k].push(Params::from_flat(&p[..n]).pos);
            }
        }
        ps[i].sub_pos = inst.sub[k].clone();
    }
    // Light(s): the lights where they are at each step's end
    // ponytail: one callback per step (a jump asks once per frame stepped through)
    if ps[ps.len() - 1].emitter_type == 4 {
        for (i, b) in ps.iter_mut().enumerate().skip(1) {
            b.lights = io.comp_lights((lo + i) as f64 / fps)?;
        }
    }
    // Lighting: the lights where they are at the frame drawn
    let last = ps.len() - 1;
    if ps[last].lighting.is_some() {
        ps[last].shade = io.shade_lights(target as f64 / fps)?;
    }
    // Shadowlets: the light named "Shadow..." (the reference's Light Naming default), ambient aside
    if ps[last].shadowlets.is_some() {
        ps[last].shadow_light = io
            .lights_named(target as f64 / fps, "Shadow")?
            .into_iter()
            .find(|l| l.kind < 3);
    }
    // Bounce off the Floor Layer and Bounce 2 / 3's layers, where EffectCraft says each is at
    // each step's end (ref BL6: a moving floor is met where it is at the frame; its own motion
    // adds nothing to the bounce). A layer at the same place at a jump's first and last steps
    // counts as still over it. Bounce 2 and 3 have no stand-in controls: without a transform
    // they stay unplaced and the particles pass them by.
    let mut floor_log = String::new();
    let slots = if last > 0 { ps[last].floors.len() } else { 0 };
    for s in 0..slots {
        let (param, layer, alpha) = {
            let f = &ps[last].floors[s];
            (f.param, f.layer, f.alpha_mode)
        };
        if !io.xf || layer < 0 {
            if s == 0 {
                floor_log += &format!("; floor from the controls (layer {layer})");
            }
            continue;
        }
        let end = io.layer_transform(param, target as f64 / fps)?;
        floor_log += &match end {
            Some((_, r)) => format!("; {param} placed (extent {r:?})"),
            None => format!("; {param}: EffectCraft gave no transform"),
        };
        let first = if last > 1 {
            io.layer_transform(param, (lo + 1) as f64 / fps)?
        } else {
            end
        };
        let still = last > 1 && first == end;
        let mut prev = if still {
            end
        } else {
            io.layer_transform(param, lo as f64 / fps)?
        };
        for (i, b) in ps.iter_mut().enumerate().skip(1) {
            let x = if i == last || still {
                end
            } else if i == 1 {
                first
            } else {
                io.layer_transform(param, (lo + i) as f64 / fps)?
            };
            if let (Some(f), Some((m, rect))) = (b.floors.get_mut(s), x) {
                f.place(&m, &prev.unwrap_or((m, rect)).0, rect);
            }
            prev = x;
        }
        // Layer Alpha: where the layer is opaque (ref BL8). The reference reads the layer's source
        // and ignores its masks (BL7); EffectCraft hands masks over too. Fetching the layer
        // costs EffectCraft ~13 ms at 1080p, so the mask is kept from render to render and
        // fetched again every FLOOR_REFETCH renders, or when the layer or frame grid changes.
        // ponytail: an animated layer's alpha shows up to FLOOR_REFETCH renders late, and is
        // one frame's over a whole jump.
        if end.is_some() && alpha {
            let key = (
                layer,
                w,
                h,
                scale.to_bits(),
                origin[0].to_bits(),
                origin[1].to_bits(),
            );
            let kept = inst.floor_mask[s]
                .as_mut()
                .filter(|c| c.0 == key && c.1 < FLOOR_REFETCH);
            let m = match kept {
                Some(c) => {
                    c.1 += 1;
                    Some(c.2.clone())
                }
                None => {
                    let t_alpha = std::time::Instant::now();
                    inst.floor_mask[s] = io
                        .layer_input(param, target as f64 / fps)?
                        .map(|(o, iw, ih)| -> Result<_, String> {
                            let px = frame_pixels(frames, view, o, iw, ih)?;
                            Ok((key, 1, std::sync::Arc::new(FloorMask::new(px, iw, ih))))
                        })
                        .transpose()?;
                    floor_log += &format!(
                        ", alpha fetched {:.1}ms",
                        t_alpha.elapsed().as_secs_f64() * 1000.0
                    );
                    inst.floor_mask[s].as_ref().map(|c| c.2.clone())
                }
            };
            for f in ps[1..].iter_mut().filter_map(|b| b.floors.get_mut(s)) {
                f.mask = m.clone();
            }
        }
    }
    let at = |k: usize| &ps[k - lo];
    let dt = (1.0 / fps) as f32;
    // Layer emitter: the layer at each step's frame (its particles' birth), or at 0 (Still).
    // EffectCraft gives every layer fetched during one render a new slot of the exchange file
    // (33 MB at 1080p), so a long jump can't fetch every frame: a layer that's the same at the
    // jump's first and last frames counts as still over it, and an animated one is fetched at
    // most MAX_FETCH times, evenly, each frame using the nearest. Playback (one frame a render)
    // is exact.
    // ponytail: edits to the emitter layer's own content aren't noticed in frames already
    // simulated (only parameters are compared); nudge a parameter to re-simulate.
    const MAX_FETCH: usize = 8;
    let emits = |k: usize| {
        let b = at(k + 1);
        b.emitter_type == 3
            && b.emit_layer >= 0
            && if b.explode {
                k == 0
            } else {
                at(k).rate + b.rate > 0.0
            }
    };
    let steps: Vec<usize> = (inst.cur.k as usize..target as usize)
        .filter(|&k| emits(k))
        .collect();
    // the frames to fetch, each with the image EC returned and a digest of it
    let mut shots: Vec<(usize, ImageRef)> = Vec::new();
    let t_fetch = std::time::Instant::now();
    let mut get = |f: usize, io: &mut Io<'_, I>| -> Result<(ImageRef, u64), String> {
        let img = io.layer_input("emitter_layer", f as f64 / fps)?;
        let dg = match img {
            Some((o, iw, ih)) => digest(frame_pixels(frames, view, o, iw, ih)?),
            None => 0,
        };
        Ok((img, dg))
    };
    if let (Some(&k0), Some(&k1)) = (steps.first(), steps.last()) {
        if at(k0 + 1).layer_still {
            if !inst.still.as_ref().is_some_and(|c| {
                c.scale == scale && c.origin == origin && c.edges == at(k0 + 1).edges
            }) {
                inst.still = None;
                if let Some((o, iw, ih)) = io.layer_input("emitter_layer", 0.0)? {
                    let px = frame_pixels(frames, view, o, iw, ih)?.to_vec();
                    let mut opaque = Vec::new();
                    let edges = at(k0 + 1).edges;
                    opaque_pixels(&px, iw, edges, &mut opaque);
                    inst.still = Some(StillLayer {
                        px,
                        w: iw,
                        h: ih,
                        opaque,
                        scale,
                        origin,
                        edges,
                    });
                }
                shots.push((0, None)); // counted in the log
            }
        } else {
            let (a, da) = get(k0 + 1, io)?;
            shots.push((k0 + 1, a));
            if k1 > k0 {
                let (b, db) = get(k1 + 1, io)?;
                if da != db {
                    let n = steps.len().min(MAX_FETCH);
                    for i in 1..n.saturating_sub(1) {
                        let f = steps[i * (steps.len() - 1) / (n - 1)] + 1;
                        shots.push((f, get(f, io)?.0));
                    }
                    shots.push((k1 + 1, b));
                }
            }
        }
    }
    let fetch_ms = t_fetch.elapsed().as_secs_f64() * 1000.0;
    let mut scan_ms = 0.0;
    let org = [origin[0] as f32, origin[1] as f32];
    let center = [layer[0] as f32 * 0.5, layer[1] as f32 * 0.5];
    let mut scanned: Option<u64> = None; // the exchange offset `opaque` was built from
    let mut xf_last: Option<(usize, [[f32; 4]; 3])> = None;
    while inst.cur.k < target {
        let k = inst.cur.k as usize;
        let b = at(k + 1);
        // the shot nearest this frame
        let shot = emits(k)
            .then(|| shots.iter().min_by_key(|s| s.0.abs_diff(k + 1)))
            .flatten();
        // the emitter layer's transform at the step's start and end (the end kept for the next)
        let xf = if emits(k) && io.xf {
            let a = match xf_last.take() {
                Some((f, m)) if f == k => Some(m),
                _ => io
                    .layer_transform("emitter_layer", k as f64 / fps)?
                    .map(|x| x.0),
            };
            let e = io
                .layer_transform("emitter_layer", (k + 1) as f64 / fps)?
                .map(|x| x.0);
            xf_last = e.map(|m| (k + 1, m));
            a.zip(e).map(|(a, e)| [a, e])
        } else {
            None
        };
        if b.layer_still && emits(k) {
            match &inst.still {
                Some(c) => {
                    let l = EmitLayer {
                        px: &c.px,
                        w: c.w,
                        h: c.h,
                        origin: org,
                        scale: scale as f32,
                        opaque: &c.opaque,
                        center,
                        xf,
                    };
                    step_with(&mut inst.cur, at(k), b, dt, Some(&l));
                }
                None => step(&mut inst.cur, at(k), b, dt),
            }
        } else {
            match shot.and_then(|s| s.1) {
                Some((o, iw, ih)) => {
                    let px = frame_pixels(frames, view, o, iw, ih)?;
                    if b.grid.is_none() && scanned != Some(o) {
                        let s0 = std::time::Instant::now();
                        opaque_pixels(px, iw, b.edges, opaque);
                        scan_ms += s0.elapsed().as_secs_f64() * 1000.0;
                        scanned = Some(o);
                    }
                    let l = EmitLayer {
                        px,
                        w: iw,
                        h: ih,
                        origin: org,
                        scale: scale as f32,
                        opaque,
                        center,
                        xf,
                    };
                    step_with(&mut inst.cur, at(k), b, dt, Some(&l));
                }
                None => step(&mut inst.cur, at(k), b, dt),
            }
        }
        if inst.cur.k.is_multiple_of(CHECKPOINT)
            && inst.checkpoints.last().is_none_or(|s| s.k < inst.cur.k)
        {
            inst.checkpoints.push(inst.cur.clone());
        }
    }
    let t2 = std::time::Instant::now();

    // Sprites. Current Time: the sprite layer at this time, every render (it may be animated or
    // edited). Other Time Sampling modes: each particle shows its own frame of the clip, so the
    // frames in use are kept from render to render, at most SPRITE_FETCH new ones fetched per
    // render (spread over the missing ones; until then a particle shows the nearest one held).
    // With nothing new to fetch, one held frame is fetched again in turn: if the sprite layer was
    // edited, the held frames are dropped and fetched anew.
    let shown = at(target as usize);
    let (mut sprite_fetches, mut sprite_io_ms) = (0, 0.0);
    let t_spr = std::time::Instant::now();
    if shown.ptype == 3 && shown.sprite_layer >= 0 {
        let key = (
            shown.sprite_layer,
            w,
            h,
            scale.to_bits(),
            origin[0].to_bits(),
            origin[1].to_bits(),
        );
        if inst.sprite_key != Some(key) || shown.time_sampling == 0 {
            inst.sprites.clear();
            inst.sprite_sums.clear();
            inst.sprite_key = Some(key);
        }
        let mut need = Vec::new();
        if shown.time_sampling == 0 {
            need.push(target as i64);
        } else {
            sprite_frames(&inst.cur, shown, (1.0 / fps) as f32, &mut need);
            need.retain(|f| !inst.sprites.has(*f));
            let room = SPRITE_HOLD
                .saturating_sub(inst.sprites.len())
                .min(SPRITE_FETCH);
            if need.len() > room {
                let m = need.len();
                need = (0..room).map(|i| need[i * m / room]).collect();
            }
            if need.is_empty() && !inst.sprites.is_empty() {
                let f = inst
                    .sprites
                    .frames()
                    .nth(inst.sprite_turn % inst.sprites.len())
                    .unwrap();
                inst.sprite_turn += 1;
                need.push(f);
            }
        }
        for f in need {
            let t_io = std::time::Instant::now();
            let img = io.layer_input("sprite_layer", f as f64 / fps)?;
            sprite_io_ms += t_io.elapsed().as_secs_f64() * 1000.0;
            sprite_fetches += 1;
            let (px, iw, ih) = match img {
                Some((o, iw, ih)) => (frame_pixels(frames, view, o, iw, ih)?, iw, ih),
                None => (&mut [][..], 0, 0),
            };
            if shown.time_sampling == 0 {
                inst.sprites.insert(f, px, iw, ih);
                continue;
            }
            let dg = digest(px);
            match inst.sprite_sums.insert(f, dg) {
                Some(old) if old == dg => continue,
                Some(_) => {
                    // the sprite layer changed: start over from this frame
                    inst.sprites.clear();
                    inst.sprite_sums.clear();
                    inst.sprite_sums.insert(f, dg);
                }
                None => {}
            }
            inst.sprites.insert(f, px, iw, ih);
        }
    } else {
        inst.sprites.clear();
        inst.sprite_sums.clear();
    }
    let sprite_ms = t_spr.elapsed().as_secs_f64() * 1000.0 - sprite_io_ms;

    // Layer Sampling Current Time: the emitter layer now, read at each particle's birth pixel
    // (one more fetch a render)
    let mut now = std::mem::take(&mut inst.now_colors);
    now.clear();
    if shown.emitter_type == 3
        && shown.emit_layer >= 0
        && shown.layer_now
        && shown.layer_color
        && let Some((o, iw, ih)) = io.layer_input("emitter_layer", time)?
    {
        layer_colors(
            &inst.cur,
            frame_pixels(frames, view, o, iw, ih)?,
            iw,
            ih,
            &mut now,
        );
    }

    // the comp's camera layer, when there is one (EffectCraft's comp_camera)
    let mut cam = io.comp_camera(None)?;
    // motion blur: the camera at the shutter's start and end too
    let bp = at(target as usize);
    if let Some(c) = cam.as_mut().filter(|_| bp.mb_samples > 1) {
        let t = target as f64 / fps;
        let (t0, len) = (bp.shutter[0] as f64 / fps, bp.shutter[1] as f64 / fps);
        let (c0, c1) = (
            io.comp_camera(Some(t + t0))?,
            io.comp_camera(Some(t + t0 + len))?,
        );
        c.shutter = c0.zip(c1).map(|(a, b)| [a.m, b.m]).filter(|[a, b]| a != b);
    }

    // a time between frames: the frame before, moved on by the fraction (particraft::between)
    let frac = (time * fps - target as f64) as f32;
    let moved = (frac > 1e-4).then(|| particraft::between(&inst.cur, frac / fps as f32));
    let px = frame_pixels(frames, view, offset, w, h)?;
    let drawn = render(
        px,
        w,
        h,
        moved.as_ref().unwrap_or(&inst.cur),
        at(target as usize),
        scale as f32,
        [origin[0] as f32, origin[1] as f32],
        [layer[0] as f32, layer[1] as f32],
        (1.0 / fps) as f32,
        scratch,
        &inst.sprites,
        (!now.is_empty()).then_some(&now[..]),
        cam.as_ref(),
        io.bounds,
    );
    inst.now_colors = now;
    // PARTICLES_LOG=<file>: one line per render with where the time went.
    if let Some(log) = std::env::var_os("PARTICLES_LOG") {
        let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f64() * 1000.0;
        let line = format!(
            "wall={} inst={} t={time:.3} frame={target} params={:.2}ms sim={:.2}ms ({} steps; emitter layer {} fetches {fetch_ms:.1}ms, scan {scan_ms:.1}ms) sprite {sprite_fetches} fetches {sprite_io_ms:.1}ms, scan {sprite_ms:.1}ms; draw={:.2}ms particles={}{floor_log}
",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()),
            req["instance"],
            ms(t0, t1),
            ms(t1, t2),
            target as usize - from.min(target as usize),
            shots.len() + usize::from(shots.len() == 1 && steps.len() > 1 && !at(steps[0] + 1).layer_still),
            ms(t2, std::time::Instant::now()),
            inst.cur.ps.len()
        );
        let _ = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut f| f.write_all(line.as_bytes()));
    }
    Ok(io.bounds.then_some(drawn))
}

/// An image in the exchange file: (offset, width, height); None when there's none.
type ImageRef = Option<(u64, usize, usize)>;

/// A quick digest of an image, to tell whether the emitter layer changed over a jump.
fn digest(px: &[Px]) -> u64 {
    use rayon::prelude::*;
    let part = |c: &[Px]| {
        c.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| {
            let v = (u64::from(p[0].to_bits()) << 32 | u64::from(p[1].to_bits()))
                ^ (u64::from(p[2].to_bits()) << 32 | u64::from(p[3].to_bits())).rotate_left(17);
            (h ^ v).wrapping_mul(0x100_0000_01b3)
        })
    };
    // in parallel chunks, combined in order
    let parts: Vec<u64> = px.par_chunks(1 << 16).map(part).collect();
    parts.iter().fold(0u64, |h, &v| {
        (h.rotate_left(5) ^ v).wrapping_mul(0x9e37_79b9_7f4a_7c15)
    })
}

fn frame_pixels<'a>(
    frames: Option<&File>,
    view: &'a mut Option<memmap2::MmapMut>,
    offset: u64,
    w: usize,
    h: usize,
) -> Result<&'a mut [Px], String> {
    let file = frames.ok_or("the frame exchange file isn't open")?;
    let n = w.checked_mul(h).filter(|&n| n > 0).ok_or("empty frame")?;
    let bytes = n.checked_mul(size_of::<Px>()).ok_or("frame too large")?;
    let end = usize::try_from(offset)
        .ok()
        .and_then(|o| o.checked_add(bytes))
        .ok_or("frame offset too large")?;
    if view.as_ref().is_none_or(|m| m.len() < end) {
        let len = file
            .metadata()
            .map_err(|e| format!("frame exchange: {e}"))?
            .len();
        if (len as usize) < end {
            return Err(format!(
                "frame ends at {end} but the exchange file has {len} bytes"
            ));
        }
        *view = None;
        // SAFETY: EffectCraft doesn't touch the frame while it waits for this render's reply,
        // and never shrinks the file.
        *view = Some(
            unsafe { memmap2::MmapOptions::new().len(len as usize).map_mut(file) }
                .map_err(|e| format!("map frames: {e}"))?,
        );
    }
    let map = view.as_mut().ok_or("no frame mapping")?;
    let frame = &mut map[end - bytes..end];
    if frame.as_ptr().align_offset(align_of::<Px>()) != 0 {
        return Err("frame isn't 4-byte aligned in the exchange file".into());
    }
    // SAFETY: n pixels of 16 bytes are exactly `frame`, which is aligned for f32 (checked above).
    Ok(unsafe { std::slice::from_raw_parts_mut(frame.as_mut_ptr().cast::<Px>(), n) })
}

fn flat(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
        .unwrap_or_default()
}
