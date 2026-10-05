// Cruor 1.0.0 - physics blood for Exanima (cast-off ropes, blood when spectating, 48 m stain area)
// level and prop collision; blood on props is tied to the exact object hit.
//
// Blood sprays from wounds as a fluid (jets that hold together, stretch and break into
// drops), lands on real surfaces with splash droplets, and stains floors, walls and
// movable objects through the game's own shaders (lit by the scene, under characters,
// moving with objects you push around).
//
// Controls:
//   F5            blood on/off (off = the game's own blood)
//   F8            clear all blood
//   F10 (hold)    blood stream in front of your character
//   Numpad 8 / 2  pick a setting (amount, force, spread, drop size, splat size,
//                 gravity, stickiness, drip seconds, darkness)
//   Numpad 4 / 6  change it (hold to change quickly; - / + also work)
//   Numpad 5      reset it
// Settings are saved to mods/Cruor/Cruor-settings.txt.

#![allow(clippy::missing_safety_doc)]
#![allow(unused_unsafe)]
#![allow(dead_code)]

use emf_rs::{macros::plugin, once_cell::sync::OnceCell, safer_ffi::prelude::char_p};
use log::*;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// log::info! + the log file
macro_rules! info_f {
    ($($t:tt)*) => {{ let __s = format!($($t)*); log::info!("{}", __s); crate::file_log(&__s); }};
}
macro_rules! error_f {
    ($($t:tt)*) => {{ let __s = format!($($t)*); log::error!("{}", __s); crate::file_log(&__s); }};
}
/// Test-only messages: silent in the release.
macro_rules! test_f {
    ($($t:tt)*) => {{ if !crate::RELEASE { info_f!($($t)*); } }};
}
macro_rules! warn_f {
    ($($t:tt)*) => {{ let __s = format!($($t)*); log::warn!("{}", __s); crate::file_log(&__s); }};
}
use std::sync::Mutex;

static FIRST_RUN: OnceCell<bool> = OnceCell::new();

// ---------------- Tuning ----------------
/// Particles per spray = spray amount / this (clamped).
const AMOUNT_PER_BLOB: f32 = 1.2;
const MIN_BLOBS: usize = 10;
const MAX_BLOBS_PER_HIT: usize = 70;
/// Launch speeds (cm/s). The jet runs from slow (back) to fast (front).
const SPEED_MIN: f32 = 120.0;
const SPEED_MAX: f32 = 480.0;
/// Sideways randomness of the jet.
const SPREAD: f32 = 0.16;
const GRAVITY: f32 = 981.0;
/// Fraction of speed lost per second to air.
const AIR_DRAG: f32 = 0.25;
/// Particle radius range (cm).
const RADIUS_MIN: f32 = 0.7;
const RADIUS_MAX: f32 = 1.5;
/// Drawn size multiplier (the camera is far away; real-size drops vanish).
const VISUAL_SCALE: f32 = 1.7;
/// Fluid behaviour: particles closer than this attract (cm).
const COHESION_RANGE: f32 = 5.0;
/// Strength of the pull between neighbouring particles (1/s^2).
const COHESION_STRENGTH: f32 = 900.0;
/// How strongly neighbours match each other's velocity (1/s) - makes ropes.
const VISCOSITY: f32 = 6.0;
/// How much a landing particle spreads into a splat.
const SPLAT_GROWTH: f32 = 3.0;
/// Splats never end up smaller than this (cm), so small ones don't blur away.
const MIN_SPLAT_RADIUS: f32 = 2.5;
const DRIPS_PER_SECOND: f32 = 6.0;
/// Flying blobs stretch along their motion by this much per cm/s.
const STRETCH: f32 = 0.004;
/// Hips are this high above the feet (cm) - only used as a fallback floor.
const HIP_HEIGHT: f32 = 88.0;
const MAX_BLOBS_TOTAL: usize = 10000;
// ----------------------------------------

/// Offset of the character's position (x, y, z) inside the character object (a physics-body
/// field: +0x360 on the public version, +0x240 on the newer beta - set with the layout).
static OFF_POS: AtomicUsize = AtomicUsize::new(0x360);

// ---- In-game tunable settings (numpad) ----
// (name, default, min, max, multiplicative step or additive step if < 0)
const TUNE: [(&str, f32, f32, f32, f32); 13] = [
    ("amount", 1.123, 0.1, 6.0, 1.15),
    ("force", 0.958, 0.1, 4.0, 1.1),
    ("spread", 5.257, 0.0, 8.0, 1.15),
    ("drop_size", 0.622, 0.2, 5.0, 1.1),
    ("splat_size", 0.69, 0.2, 5.0, 1.1),
    ("gravity", 0.999, 0.0, 4.0, 1.1),
    ("stickiness", 0.115, 0.0, 5.0, 1.15),
    ("drip_seconds", 15.0, 0.0, 30.0, -0.5),
    ("darkness", 0.9, 0.2, 1.8, -0.05),
    // how much a wound drips (1 = 6 drops a second)
    ("drip_amount", 2.615, 0.0, 8.0, 1.15),
    // how much a hit's hardness changes the spray (0 = every hit alike, 1 = normal, 2 = strong)
    ("hit_impact", 1.0, 0.0, 2.0, -0.1),
    // how much blood a bloody weapon flings off its tip when swung (0 = off)
    ("cast_off", 0.25, 0.0, 3.0, -0.25),
    // how stringy the blood flung off a weapon is - its own stickiness (0 = loose drops)
    ("cast_off_string", 0.25, 0.0, 5.0, -0.25),
];
const T_AMOUNT: usize = 0;
const T_FORCE: usize = 1;
const T_SPREAD: usize = 2;
const T_DROP: usize = 3;
const T_SPLAT: usize = 4;
const T_GRAVITY: usize = 5;
const T_STICKY: usize = 6;
const T_DRIP: usize = 7;
const T_DARK: usize = 8;
const T_DRIP_AMT: usize = 9;
const T_IMPACT: usize = 10;
const T_CASTOFF: usize = 11;
const T_CO_STRING: usize = 12;
static TUNE_VALUES: Mutex<[f32; 13]> = Mutex::new([1.123, 0.958, 5.257, 0.622, 0.69, 0.999, 0.115, 15.0, 0.9, 2.615, 1.0, 0.25, 0.25]);
static TUNE_SELECTED: AtomicU32 = AtomicU32::new(0);
fn tv(i: usize) -> f32 {
    TUNE_VALUES.lock().map(|v| v[i]).unwrap_or(TUNE[i].1)
}

// Hotkey-controlled switches.
static BLOOD_ON: AtomicBool = AtomicBool::new(true);
static DELAY_ON: AtomicBool = AtomicBool::new(true);
static DEPTH_ON: AtomicBool = AtomicBool::new(true);
static FLUID_ON: AtomicBool = AtomicBool::new(true);

static RNG: AtomicU32 = AtomicU32::new(0x9E37_79B9);
fn rand01() -> f32 {
    let mut x = RNG.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    RNG.store(x, Ordering::Relaxed);
    x as f32 / u32::MAX as f32
}
fn rand_range(a: f32, b: f32) -> f32 {
    a + (b - a) * rand01()
}
fn norm3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-6 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 0.0, 0.0] }
}


// ============================================================
// Fluid simulation
// ============================================================
#[derive(Clone, Copy)]
struct Blob {
    pos: [f32; 3],
    vel: [f32; 3],
    radius: f32,
    floor_y: f32,
    landed: bool,
    fresh: bool,
    age: f32,
    group: u32,
    /// Ignore surfaces until this age (set while running off a body).
    skip_until: f32,
    /// Landed on a surface we actually saw (true) or on the backup floor (false).
    exact: bool,
    /// Already painted into the stain maps?
    mapped: bool,
    /// Where its path was last collision-tested from (NaN = not yet).
    tested_from: [f32; 3],
    /// Height band (cm) its floor stain is drawn within; 0 = the normal tight band.
    /// Wider when it landed on a sloped collision surface, e.g. the hidden ramp the
    /// game uses for stairs, so the stain shows on the steps that ramp runs through.
    band: f32,
    /// Pooling: radius (cm) this puddle still has to spread out by.
    grow: f32,
    /// Splat shape from the impact: travel direction on the floor (x, z), stretch
    /// (1 = round, up to 2 for a grazing hit) and spikiness (0..1, fast impacts).
    shape: [f32; 4],
    /// Cast-off strings: this drop's place along the string (springs join n and n+1 only).
    seq: u32,
}
/// A round splat with no direction (stretch 1, no spikes).
const NO_SHAPE: [f32; 4] = [1.0, 0.0, 1.0, 0.0];

struct Drip {
    motile: usize,
    offset: [f32; 3],
    time_left: f32,
    accum: f32,
    /// how heavily it drips (harder hits drip more)
    rate: f32,
}

/// A splat stuck to a wall: centre, which way the wall faces, size, and a random seed.
#[derive(Clone, Copy)]
struct WallSplat {
    pos: [f32; 3],
    normal: [f32; 3],
    radius: f32,
    seed: f32,
    mapped: bool,
    /// A streak segment: length (cm) it runs straight down from pos (0 = a round splat).
    streak: f32,
    /// Splat shape: travel direction across the wall (u, v), stretch, spikiness.
    shape: [f32; 4],
}
const MAX_WALL_SPLATS: usize = 4000;

/// DRIPPING: a bead of blood running down a wall, leaving a thinning streak. It follows
/// the wall using the game's collision, falls off as a drop where the wall ends, and
/// feeds a floor pool where it reaches the floor.
#[derive(Clone, Copy)]
struct Runner {
    pos: [f32; 3],
    normal: [f32; 3],
    /// Streak width at the start (cm).
    r0: f32,
    /// Streak length it can still run (cm), and its full length.
    left: f32,
    len: f32,
    speed: f32,
    max_speed: f32,
    since_deposit: f32,
    since_check: f32,
}
const MAX_RUNNERS: usize = 150;
/// Wall splats at least this wide (cm) can start a runner.
const RUNNER_MIN_SPLAT: f32 = 1.0;
const RUNNER_CHANCE: f32 = 0.6;
const RUNNER_ACCEL: f32 = 6.0; // cm/s^2
/// POOLING: puddles grow to at most this radius (cm), and spread out at this rate.
const POOL_MAX_RADIUS: f32 = 90.0;
const POOL_SPREAD_RATE: f32 = 1.2; // fraction of the remaining spread per second

struct Sim {
    walls: Vec<WallSplat>,
    runners: Vec<Runner>,
    blobs: Vec<Blob>,
    drips: Vec<Drip>,
    last_update: Option<std::time::Instant>,
    next_group: u32,
}

static SIM: Mutex<Sim> = Mutex::new(Sim { walls: Vec::new(), runners: Vec::new(), blobs: Vec::new(), drips: Vec::new(), last_update: None, next_group: 1 });

static DEPTH_LANDINGS: AtomicU32 = AtomicU32::new(0);
static FLOOR_LANDINGS: AtomicU32 = AtomicU32::new(0);
static BODY_HITS: AtomicU32 = AtomicU32::new(0);
/// How far the real floor is from our hip-based estimate (cm), learned from
/// real landings. (bias, number of samples)
static FLOOR_BIAS: Mutex<(f32, u32)> = Mutex::new((0.0, 0));

fn floor_bias() -> (f32, bool) {
    FLOOR_BIAS.lock().map(|g| (g.0, g.1 >= 3)).unwrap_or((0.0, false))
}

/// Recent real floor heights (from drops that landed on floor we could see).
static FLOOR_SAMPLES: Mutex<Vec<[f32; 3]>> = Mutex::new(Vec::new());
/// The real floor height near (x, z), if we've measured floor within 3 m.
fn local_floor(x: f32, z: f32) -> Option<f32> {
    let v = FLOOR_SAMPLES.lock().ok()?;
    let mut best: Option<(f32, f32)> = None;
    for p in v.iter() {
        let d2 = (p[0] - x).powi(2) + (p[2] - z).powi(2);
        if d2 < 300.0 * 300.0 && best.map(|b| d2 < b.0).unwrap_or(true) {
            best = Some((d2, p[1]));
        }
    }
    best.map(|b| b.1)
}

unsafe fn read_hip(motile: usize) -> [f32; 3] {
    let p = unsafe { (motile as *const u8).add(OFF_POS.load(Ordering::Relaxed)) } as *const f32;
    unsafe { [p.read_unaligned(), p.add(1).read_unaligned(), p.add(2).read_unaligned()] }
}

fn start_drip(motile: usize, wound: [f32; 3], hard: f32) {
    if motile == 0 {
        return;
    }
    let hip = unsafe { read_hip(motile) };
    let mut sim = match SIM.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    if sim.drips.len() > 24 {
        sim.drips.remove(0);
    }
    sim.drips.push(Drip {
        motile,
        offset: [wound[0] - hip[0], wound[1] - hip[1], wound[2] - hip[2]],
        time_left: tv(T_DRIP) * impact(hard.powf(0.6)),
        accum: 0.0,
        rate: impact(hard.sqrt()),
    });
}

/// Launch a jet of blood from a wound. Particles are laid out along the jet
/// from slow (back) to fast (front) so cohesion pulls them into a rope that
/// stretches and breaks up into droplets.
/// CAST-OFF: blood flung off a bloody weapon's tip as it's swung.
/// The game keeps its own list of bloody weapons (TWeaponBlood, one per scene, global at
/// 0x1004EF870 in the beta): its hit code (0x10015C640) takes the attacker's weapon
/// ([character+0x988]), gives its node ([weapon+0x18]) a slot (0x100100520) and adds blood
/// (0x1001006A0, capped at 1). The list: count +0x8, 24-byte entries at +0x10 - entry +0x0 the
/// weapon node, +0x8 its blood (0..1). Each frame the plugin reads that list; when a weapon's
/// blood goes up it gets drops to fling, and while its tip moves fast it releases them with the
/// tip's own velocity.
/// Where the weapon is: the game draws each bloody weapon (0x1001008B4..) with
/// ObjMat = camera * A, A = the 4x4 at node+0xA0 (0x100018C70, uploaded untransposed), and its
/// shader does gl_Position = ObjMat * vec4(vPos, 1) - so a weapon-space point p is placed at
/// A * p, A column-major (translation at floats 12-14). It only draws weapons with node+0xE0 bit
/// 0x1000000. The tip (an estimate, checked in the log): the far point of the weapon's own
/// bounding sphere (+0x10 centre, +0x2C radius, weapon coordinates) from its origin.
const WEAPON_BLOOD_SIG: &str = "48 8B 0D ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 8B 55 F8 48 8B 92 ?? ?? 00 00 89 82 ?? ?? 00 00";
static WEAPON_BLOOD_GLOBAL: AtomicUsize = AtomicUsize::new(0);
static WEAPON_BLOOD_SEARCHED: AtomicBool = AtomicBool::new(false);
const CAST_OFF_MIN_SPEED: f32 = 350.0;
/// Cast-off drops of one swing share a group in this range, so the jet springs hold them
/// together as a rope - with the cast_off_string setting, not the base stickiness.
const CAST_OFF_GROUP: u32 = 0x4000_0000;
static CAST_OFF_NEXT_GROUP: AtomicU32 = AtomicU32::new(1);
/// Drops laid along the tip's path about this far apart.
const CAST_OFF_SPACING: f32 = 3.0;
/// At most this many drops leave a blade per second (a hit's blood trails through the swing;
/// the F6 test can't flood the simulation).
const CAST_OFF_RATE: f32 = 250.0;
struct CastOffWeapon {
    node: usize,
    blood: f32,
    tip: [f32; 3],
    have_tip: bool,
    budget: f32,
    carry: f32,
    drip: f32,
    rope: u32,
    rope_n: u32,
    test: bool,
}
static CAST_OFF: Mutex<Vec<CastOffWeapon>> = Mutex::new(Vec::new());
/// F6 test: the player's weapon always has blood - drips from the tip and flings when swung.
static DRIP_TEST: AtomicBool = AtomicBool::new(false);
static DRIP_TEST_LOGGED: AtomicBool = AtomicBool::new(false);
/// The character's weapon field, read from the game's hit code (0x988 beta).
static WEAPON_FIELD: AtomicUsize = AtomicUsize::new(0);
static CAST_OFF_DROPS: AtomicU32 = AtomicU32::new(0);
static CAST_OFF_LOGS: AtomicU32 = AtomicU32::new(0);
static CAST_OFF_REPORT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// TEST: blood simulation time per frame (sum, worst, frames) and flying drops, for the report.
static SIM_TIME: Mutex<(f64, f64, u32)> = Mutex::new((0.0, 0.0, 0));
static FLYING_NOW: AtomicU32 = AtomicU32::new(0);
static VERT_TIME: Mutex<(f64, f64, u32)> = Mutex::new((0.0, 0.0, 0));

fn weapon_blood_manager() -> usize {
    if !WEAPON_BLOOD_SEARCHED.swap(true, Ordering::Relaxed) {
        match unsafe { find_pattern(WEAPON_BLOOD_SIG) } {
            Some(m) => {
                let disp = unsafe { ((m + 3) as *const i32).read_unaligned() };
                let g = (m as isize + 7 + disp as isize) as usize;
                WEAPON_BLOOD_GLOBAL.store(g, Ordering::Relaxed);
                WEAPON_FIELD.store(unsafe { ((m + 19) as *const u32).read_unaligned() } as usize, Ordering::Relaxed);
                test_f!("Cruor TEST: cast-off - the game's bloody-weapon list is at 0x{:X} (read from its hit code at 0x{:X})", g, m);
            }
            None => test_f!("Cruor TEST: cast-off - the game's bloody-weapon list wasn't found; cast-off stays off"),
        }
    }
    let g = WEAPON_BLOOD_GLOBAL.load(Ordering::Relaxed);
    if g == 0 {
        return 0;
    }
    // (a global in the exe image: always there)
    unsafe { (g as *const usize).read_unaligned() }
}

fn player_weapon_node() -> usize {
    let wf = WEAPON_FIELD.load(Ordering::Relaxed);
    let slot = PLAYER_SLOT.load(Ordering::Relaxed);
    if wf == 0 || slot == 0 {
        return 0;
    }
    let player = unsafe { (slot as *const usize).read_unaligned() };
    if player < 0x10000 || !readable(player + wf, 8) {
        return 0;
    }
    let weapon = unsafe { ((player + wf) as *const usize).read_unaligned() };
    if weapon < 0x10000 || !readable(weapon + 0x18, 8) {
        return 0;
    }
    let node = unsafe { ((weapon + 0x18) as *const usize).read_unaligned() };
    if node < 0x10000 || !readable(node, 0xE8) { 0 } else { node }
}

/// F6: toggle the weapon drip test.
fn drip_test_poll() {
    if RELEASE {
        return; // (F6 weapon drip test: test builds only)
    }
    static K6: AtomicBool = AtomicBool::new(false);
    let f6 = unsafe { GetAsyncKeyState(0x75) } as u16 & 0x8000 != 0;
    if f6 && !K6.swap(true, Ordering::Relaxed) {
        let v = !DRIP_TEST.load(Ordering::Relaxed);
        DRIP_TEST.store(v, Ordering::Relaxed);
        DRIP_TEST_LOGGED.store(false, Ordering::Relaxed);
        ui_say(&format!("Cruor: weapon drip test {}", if v { "ON" } else { "OFF" }), UI_WHITE);
        test_f!("Cruor TEST: F6 - weapon drip test {}", if v { "ON" } else { "OFF" });
    } else if !f6 {
        K6.store(false, Ordering::Relaxed);
    }
}

pub(crate) fn forget_cast_off() {
    if let Ok(mut v) = CAST_OFF.lock() {
        v.clear();
    }
}

/// Once a frame (drawing thread, live level only).
fn cast_off_step(sim: &mut Sim, dt: f32) {
    let k = tv(T_CASTOFF);
    if k <= 0.0 || dt <= 0.0 || !WORLD_LIVE.load(Ordering::Relaxed) || !BLOOD_ON.load(Ordering::Relaxed) {
        return;
    }
    // The game's own rule for its list (its weapon-blood drawing, 0x100100890..0x1001008B7, every
    // frame): an entry with blood > 0.01 and a node is read straight away - node+0xE0, and its
    // matrix at +0xA0 - with no other check. Cast-off reads those entries the same way (no
    // VirtualQuery: ~50 us each here).
    let mgr = weapon_blood_manager();
    let mut list: Vec<(usize, f32)> = Vec::new();
    let mut n = 0;
    if mgr >= 0x10000 {
        n = unsafe { ((mgr + 8) as *const i32).read_unaligned() };
        let arr = unsafe { ((mgr + 0x10) as *const usize).read_unaligned() };
        if n > 0 && n <= 256 && arr >= 0x10000 {
            for i in 0..n as usize {
                let e = arr + i * 0x18;
                let node = unsafe { (e as *const usize).read_unaligned() };
                let blood = unsafe { ((e + 8) as *const f32).read_unaligned() };
                if node >= 0x10000 && blood.is_finite() && blood as f64 > 0.01 {
                    list.push((node, blood));
                }
            }
        }
    }
    let test = DRIP_TEST.load(Ordering::Relaxed);
    let pnode = if test { player_weapon_node() } else { 0 };
    if test && !DRIP_TEST_LOGGED.swap(true, Ordering::Relaxed) {
        test_f!("Cruor TEST: drip test - your weapon: {} (field +0x{:X} from the game's hit code){}",
            if pnode == 0 { "none found".to_string() } else { format!("node 0x{:X} ({})", pnode, obj_class_name(pnode)) },
            WEAPON_FIELD.load(Ordering::Relaxed),
            if pnode == 0 { String::new() } else if list.iter().any(|e| e.0 == pnode) { "; it IS in the game's bloody-weapon list (same node the game marks bloody)".into() } else { "; not in the game's bloody-weapon list right now".into() });
    }
    if pnode != 0 && !list.iter().any(|e| e.0 == pnode) {
        list.push((pnode, 1.0));
    }
    if list.is_empty() {
        forget_cast_off();
        return;
    }
    let Ok(mut ws) = CAST_OFF.lock() else { return };
    let mut seen: Vec<usize> = Vec::new();
    for &(node, blood) in list.iter() {
        // only the F6 test weapon (not from the game's list) needs memory checks
        let from_game = node != pnode || !test;
        if !from_game && (!readable(node, 0x70) || !readable(node + 0xA0, 0x48)) {
            continue;
        }
        seen.push(node);
        if unsafe { ((node + 0xE0) as *const u32).read_unaligned() } & 0x0100_0000 == 0 {
            continue; // the game doesn't draw this weapon now
        }
        let a: [f32; 16] = unsafe { ((node + 0xA0) as *const [f32; 16]).read_unaligned() };
        let place = |p: [f32; 3]| [
            a[0] * p[0] + a[4] * p[1] + a[8] * p[2] + a[12],
            a[1] * p[0] + a[5] * p[1] + a[9] * p[2] + a[13],
            a[2] * p[0] + a[6] * p[1] + a[10] * p[2] + a[14],
        ];
        let (centre, radius) = unsafe { (rv3(node + 0x10), ((node + 0x2C) as *const f32).read_unaligned()) };
        let axis = norm3(centre);
        if axis == [0.0, 0.0, 0.0] || !radius.is_finite() || radius <= 0.0 || radius > 400.0 || a.iter().any(|x| !x.is_finite()) {
            continue;
        }
        let tip_local = [centre[0] + axis[0] * radius, centre[1] + axis[1] * radius, centre[2] + axis[2] * radius];
        let grip = place([0.0, 0.0, 0.0]);
        let tip = place(tip_local);
        let w = match ws.iter().position(|w| w.node == node) {
            Some(j) => &mut ws[j],
            None => {
                if CAST_OFF_LOGS.fetch_add(1, Ordering::Relaxed) < 20 {
                    let dg = sub3(tip, grip);
                    let o64 = unsafe { rv3(node + 0x64) };
                    let me = player_hips();
                    test_f!("Cruor TEST: cast-off - bloody weapon node 0x{:X} ({}): blood {:.2}; game's draw matrix puts its origin at ({:.0}, {:.0}, {:.0}) [node +0x64 says ({:.0}, {:.0}, {:.0}); you are at {}]; own bounds centre ({:.0}, {:.0}, {:.0}) radius {:.0} -> tip at ({:.0}, {:.0}, {:.0}), {:.0} cm from the origin",
                        node, obj_class_name(node), blood, grip[0], grip[1], grip[2], o64[0], o64[1], o64[2],
                        match me { Some(p) => format!("({:.0}, {:.0}, {:.0})", p[0], p[1], p[2]), None => "unknown".into() },
                        centre[0], centre[1], centre[2], radius, tip[0], tip[1], tip[2], dot3(dg, dg).sqrt());
                }
                ws.push(CastOffWeapon { node, blood, tip, have_tip: false, budget: 0.0, carry: 0.0, drip: 0.0, rope: 0, rope_n: 0, test: false });
                ws.last_mut().unwrap()
            }
        };
        // the game added blood (a hit): that much more to fling (a full 1.0 = 800 drops)
        if blood > w.blood + 0.001 {
            w.budget = (w.budget + (blood - w.blood) * 800.0 * k).min(800.0 * k);
        }
        w.blood = blood;
        let v = [(tip[0] - w.tip[0]) / dt, (tip[1] - w.tip[1]) / dt, (tip[2] - w.tip[2]) / dt];
        let had = w.have_tip;
        let prev_tip = w.tip;
        w.tip = tip;
        w.have_tip = true;
        // F6 test weapon: unlimited blood, and a steady drip from the tip
        if node == pnode {
            w.test = true;
            w.budget = 1.0e6;
            w.drip += dt * 6.0;
            let ok_v = had && dot3(v, v).sqrt() < 6000.0;
            while w.drip >= 1.0 {
                w.drip -= 1.0;
                let jit = [rand01() - 0.5, rand01() - 0.5, rand01() - 0.5];
                let carry_v = if ok_v { v } else { [0.0; 3] };
                let group = sim.next_group;
                sim.next_group = sim.next_group.wrapping_add(1).max(1);
                sim.blobs.push(Blob {
                    pos: [tip[0] + jit[0], tip[1] + jit[1], tip[2] + jit[2]],
                    vel: [carry_v[0] + jit[0] * 10.0, carry_v[1] - 20.0, carry_v[2] + jit[2] * 10.0],
                    radius: rand_range(0.7, 1.1),
                    floor_y: tip[1] - 300.0,
                    landed: false,
                    fresh: false,
                    age: BODY_GRACE,
                    group,
                    skip_until: BODY_GRACE + 0.06,
                    exact: false,
                    mapped: false,
                    tested_from: [f32::NAN; 3],
                    band: 0.0,
                    grow: 0.0,
                    shape: NO_SHAPE,
                    seq: 0,
                });
                CAST_OFF_DROPS.fetch_add(1, Ordering::Relaxed);
            }
        } else if w.test {
            // F6 turned off: the unlimited blood goes with it
            w.test = false;
            w.budget = 0.0;
        }
        if !had || w.budget < 1.0 {
            w.rope = 0;
            w.carry = 0.0;
            continue;
        }
        let vs = dot3(v, v).sqrt();
        if vs < CAST_OFF_MIN_SPEED || vs > 6000.0 {
            w.rope = 0; // the swing ended: the next one starts a new rope
            w.carry = 0.0;
            continue;
        }
        // how many may leave the blade this frame (the rate cap)
        w.carry = (w.carry + dt * CAST_OFF_RATE).min(16.0);
        // one rope per swing: drops laid evenly along the tip's path this frame, all moving
        // with the tip, tied together by the jet springs
        let path = sub3(tip, prev_tip);
        let len = dot3(path, path).sqrt();
        let n = ((len / CAST_OFF_SPACING).ceil() as u32).clamp(1, 16).min(w.budget as u32).min(w.carry as u32);
        if n == 0 {
            continue;
        }
        w.carry -= n as f32;
        for j in 0..n {
            let t = (j + 1) as f32 / n as f32;
            let at = [prev_tip[0] + path[0] * t, prev_tip[1] + path[1] * t, prev_tip[2] + path[2] * t];
            let f = rand_range(0.98, 1.02);
            // one string per swing, numbered along its length
            if w.rope == 0 {
                w.rope = CAST_OFF_GROUP | (CAST_OFF_NEXT_GROUP.fetch_add(1, Ordering::Relaxed) & 0x3FFF_FFFF).max(1);
                w.rope_n = 0;
            }
            let my_seq = w.rope_n;
            w.rope_n += 1;
            sim.blobs.push(Blob {
                pos: at,
                vel: [v[0] * f, v[1] * f, v[2] * f],
                radius: rand_range(0.7, 0.9),
                floor_y: tip[1] - 300.0,
                landed: false,
                fresh: false,
                age: BODY_GRACE,
                group: w.rope,
                skip_until: BODY_GRACE + 0.06,
                exact: false,
                mapped: false,
                tested_from: [f32::NAN; 3],
                band: 0.0,
                grow: 0.0,
                shape: NO_SHAPE,
                seq: my_seq,
            });
        }
        w.budget -= n as f32;
        CAST_OFF_DROPS.fetch_add(n, Ordering::Relaxed);
    }
    ws.retain(|w| seen.contains(&w.node));
    drop(ws);
    // a line every 5 s while it's flinging
    if let Ok(mut r) = CAST_OFF_REPORT.lock() {
        let due = r.map(|t| t.elapsed().as_secs_f32() >= 5.0).unwrap_or(true);
        if due {
            *r = Some(std::time::Instant::now());
            let d = CAST_OFF_DROPS.swap(0, Ordering::Relaxed);
            let (sum, worst, frames) = (0.0f64, 0.0f64, 0u32);
            let _ = (sum, worst, frames);
            if d > 0 {
                test_f!("Cruor TEST: cast-off - {} drops flung off weapon tips in 5 s; {} bloody weapons in the game's list; {} drops flying now",
                    d, n, FLYING_NOW.load(Ordering::Relaxed));
            }
        }
    }
}

/// HIT HARDNESS: the game's spray amount compared with a running average of recent hits
/// (1 = an average hit; clamped 0.35..2.5), so it calibrates itself to the game's numbers.
static HIT_AVG: Mutex<f32> = Mutex::new(0.0);
fn hit_hardness(amount: f32, dir_len: f32) -> f32 {
    let Ok(mut avg) = HIT_AVG.lock() else { return 1.0 };
    if !(amount > 0.0) {
        return 1.0;
    }
    if *avg <= 0.0 {
        *avg = amount;
    }
    let s = (amount / *avg).clamp(0.35, 2.5);
    let before = *avg;
    *avg = *avg * 0.9 + amount * 0.1;
    if !RELEASE {
        test_f!("Cruor TEST: hit amount {:.2} (running average {:.2}) -> hardness {:.2}; spray direction length {:.2}", amount, before, s, dir_len);
    }
    s
}
/// A hardness effect, scaled by the hit_impact setting (f = the full effect).
fn impact(f: f32) -> f32 {
    1.0 + (f - 1.0) * tv(T_IMPACT)
}

fn spawn_jet(pos: [f32; 3], dir: [f32; 3], amount: f32, floor_y: f32, hard: f32) {
    let amt = tv(T_AMOUNT);
    let n = ((amount * amt / AMOUNT_PER_BLOB) as usize).clamp(MIN_BLOBS, ((MAX_BLOBS_PER_HIT as f32) * amt.max(1.0)) as usize);
    // harder hits: faster, wider, slightly bigger drops
    let spread = SPREAD * tv(T_SPREAD) * impact(0.75 + 0.25 * hard);
    let force = tv(T_FORCE) * impact(hard.sqrt());
    let drop = tv(T_DROP) * impact(hard.powf(0.25));
    let mut d = norm3(dir);
    if d == [0.0, 0.0, 0.0] {
        d = [0.0, 1.0, 0.0];
    }
    let mut sim = match SIM.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    let group = sim.next_group;
    sim.next_group = sim.next_group.wrapping_add(1).max(1);
    // One slight random curve for the whole jet so it isn't a perfect line.
    let bend = [(rand01() - 0.5) * spread, (rand01() - 0.2) * spread, (rand01() - 0.5) * spread];
    for k in 0..n {
        let t = if n > 1 { k as f32 / (n - 1) as f32 } else { 1.0 };
        let dirk = norm3([
            d[0] + bend[0] * t + (rand01() - 0.5) * spread * 0.5,
            d[1] + 0.12 + bend[1] * t + (rand01() - 0.5) * spread * 0.5,
            d[2] + bend[2] * t + (rand01() - 0.5) * spread * 0.5,
        ]);
        let speed = (SPEED_MIN + (SPEED_MAX - SPEED_MIN) * t) * rand_range(0.9, 1.1) * force;
        sim.blobs.push(Blob {
            pos: [pos[0] + d[0] * t * 1.5, pos[1] + d[1] * t * 1.5, pos[2] + d[2] * t * 1.5],
            vel: [dirk[0] * speed, dirk[1] * speed, dirk[2] * speed],
            radius: rand_range(RADIUS_MIN, RADIUS_MAX) * drop,
            floor_y,
            landed: false,
            fresh: false,
            age: 0.0,
            group,
            skip_until: 0.0,
            exact: false,
            mapped: false,
            tested_from: [f32::NAN; 3],
            band: 0.0,
            grow: 0.0,
            shape: NO_SHAPE,
            seq: 0,
        });
    }
    if sim.blobs.len() > MAX_BLOBS_TOTAL {
        let extra = sim.blobs.len() - MAX_BLOBS_TOTAL;
        sim.blobs.drain(0..extra);
    }
}

/// Small droplets thrown off by impacts, added to the simulation next frame.
static SATELLITES: Mutex<Vec<Blob>> = Mutex::new(Vec::new());

fn land(b: &mut Blob, at: [f32; 3]) {
    // Splash: a fast drop throws off a few small droplets around the impact,
    // spread along the way it was travelling.
    let hs = (b.vel[0] * b.vel[0] + b.vel[2] * b.vel[2]).sqrt();
    let speed = (hs * hs + b.vel[1] * b.vel[1]).sqrt();
    // the splat's shape: a shallow drop leaves a teardrop stretched along its path,
    // a fast one a spiky edge
    // small fast drops can draw long needle lines; big ones stay stubbier
    let cap = if b.radius < 0.9 { 6.0 } else { 4.0 };
    let stretch = ((1.0 + 1.6 * hs / (b.vel[1].abs() + 60.0)) * (1.0 + ((speed - 300.0) / 600.0).clamp(0.0, 1.0)) * rand_range(0.85, 1.15)).clamp(1.0, cap);
    // a big fast drop breaks into fingers along its front edge; a small steep one
    // gets a slightly ragged rim
    let spike = if b.radius > 1.1 && speed > 250.0 {
        ((speed - 250.0) / 500.0).clamp(0.0, 1.0) * 0.8 * rand_range(0.6, 1.0)
    } else if stretch < 1.5 {
        ((speed - 400.0) / 600.0).clamp(0.0, 1.0) * 0.5 * rand01()
    } else {
        0.0
    };
    let fdir = if hs > 1.0 { [b.vel[0] / hs, b.vel[2] / hs] } else { [1.0, 0.0] };
    // a cast-off rope moves sideways to its length: its drops land round (2 cm apart, they
    // overlap into a line) and throw no splash droplets
    let rope = b.group != u32::MAX && b.group & CAST_OFF_GROUP != 0;
    let shape = if rope { NO_SHAPE } else if hs > 1.0 { [fdir[0], fdir[1], stretch, spike] } else { [1.0, 0.0, 1.0, spike] };
    if !rope && speed > 120.0 && b.group != u32::MAX {
        let n = ((speed / 150.0) as usize).clamp(1, 4) + if stretch > 2.0 { 2 } else { 0 };
        let dir = if hs > 1.0 { [b.vel[0] / hs, b.vel[2] / hs] } else { [rand01() - 0.5, rand01() - 0.5] };
        if let Ok(mut sats) = SATELLITES.lock() {
            if sats.len() < 400 {
                // a second overlapping lobe on some bigger splats
                if b.radius > 1.0 && rand01() < 0.3 {
                    let off = b.radius * SPLAT_GROWTH * tv(T_SPLAT) * rand_range(0.4, 0.7);
                    let a = rand01() * 6.283;
                    sats.push(Blob {
                        pos: [at[0] + a.cos() * off, at[1], at[2] + a.sin() * off],
                        vel: [0.0; 3],
                        radius: b.radius * rand_range(0.5, 0.75) * SPLAT_GROWTH * tv(T_SPLAT),
                        floor_y: b.floor_y,
                        landed: true,
                        fresh: false,
                        age: 0.0,
                        group: u32::MAX,
                        skip_until: 0.0,
                        exact: b.exact,
                        mapped: false,
                        tested_from: [f32::NAN; 3],
                        band: b.band,
                        grow: 0.0,
                        shape: [fdir[0], fdir[1], 1.0 + (stretch - 1.0) * 0.6, 0.0],
                        seq: 0,
                    });
                }
                for _ in 0..n {
                    let dist = b.radius * rand_range(2.0, 6.0) + rand01() * 4.0;
                    // a grazing drop throws its droplets out in a line ahead of it
                    let side = (rand01() - 0.5) * 1.2 * (1.0 - (stretch - 1.0) * 0.4).max(0.12);
                    let dx = dir[0] - dir[1] * side;
                    let dz = dir[1] + dir[0] * side;
                    // each droplet is a little teardrop pointing away from the impact,
                    // longer the further out it flew
                    let dn = (dx * dx + dz * dz).sqrt().max(1e-3);
                    let sat_shape = [dx / dn, dz / dn, (rand_range(1.3, 2.0) * (1.0 + dist / (b.radius * 16.0 + 1.0))).min(3.0), 0.0];
                    sats.push(Blob {
                        pos: [at[0] + dx * dist, at[1], at[2] + dz * dist],
                        vel: [0.0; 3],
                        radius: (b.radius * rand_range(0.35, 0.6)).max(1.0) * SPLAT_GROWTH * tv(T_SPLAT),
                        floor_y: b.floor_y,
                        landed: true,
                        fresh: false,
                        age: 0.0,
                        group: u32::MAX, // (satellites don't splash again)
                        skip_until: 0.0,
                        exact: b.exact,
                        mapped: false,
                        tested_from: [f32::NAN; 3],
                        band: b.band,
                        grow: 0.0,
                        shape: sat_shape,
                        seq: 0,
                    });
                }
                // a fast impact also scatters a fine mist of specks further out
                if speed > 300.0 {
                    let m = 2 + (rand01() * 4.0) as usize;
                    let base = dir[1].atan2(dir[0]);
                    for _ in 0..m {
                        // specks fly on ahead, within about 40 degrees of the drop's path
                        let a = base + (rand01() - 0.5) * 1.4;
                        let (cx, cz) = (a.cos(), a.sin());
                        let cn = 1.0;
                        let dist = b.radius * rand_range(4.0, 12.0) + rand01() * 6.0;
                        sats.push(Blob {
                            pos: [at[0] + cx / cn * dist, at[1], at[2] + cz / cn * dist],
                            vel: [0.0; 3],
                            radius: rand_range(0.4, 0.7) * SPLAT_GROWTH * tv(T_SPLAT),
                            floor_y: b.floor_y,
                            landed: true,
                            fresh: false,
                            age: 0.0,
                            group: u32::MAX,
                            skip_until: 0.0,
                            exact: b.exact,
                            mapped: false,
                            tested_from: [f32::NAN; 3],
                            band: b.band,
                            grow: 0.0,
                            shape: [cx / cn, cz / cn, rand_range(1.0, 1.6), 0.0],
                            seq: 0,
                        });
                    }
                }
            }
        }
    }
    b.mapped = false;
    b.exact = true;
    b.pos = at;
    b.vel = [0.0; 3];
    b.landed = true;
    b.fresh = true;
    b.shape = shape;
    b.radius = (b.radius * SPLAT_GROWTH * tv(T_SPLAT)).max(MIN_SPLAT_RADIUS * tv(T_SPLAT));
}

fn step_sim(sim: &mut Sim, _depth_works: bool) {
    set_stage(1);
    let now = std::time::Instant::now();
    let dt = match sim.last_update {
        Some(t) => (now - t).as_secs_f32().min(0.05),
        None => 0.0,
    };
    sim.last_update = Some(now);
    if dt <= 0.0 {
        return;
    }

    let t_co = std::time::Instant::now();
    cast_off_step(sim, dt);
    cost_note(7, t_co.elapsed().as_secs_f64() * 1000.0, 0);
    let t_drips = std::time::Instant::now();
    // Dripping wounds follow the character around.
    let mut new_drops: Vec<Blob> = Vec::new();
    for d in sim.drips.iter_mut() {
        d.time_left -= dt;
        d.accum += dt * DRIPS_PER_SECOND * tv(T_DRIP_AMT) * d.rate * (d.time_left / tv(T_DRIP).max(0.1)).clamp(0.2, 1.0);
        while d.accum >= 1.0 {
            d.accum -= 1.0;
            let hip = unsafe { read_hip(d.motile) };
            if !hip.iter().all(|v| v.is_finite()) {
                d.time_left = 0.0;
                break;
            }
            new_drops.push(Blob {
                pos: [hip[0] + d.offset[0], hip[1] + d.offset[1], hip[2] + d.offset[2]],
                vel: [(rand01() - 0.5) * 25.0, -rand01() * 30.0, (rand01() - 0.5) * 25.0],
                radius: rand_range(RADIUS_MIN * 0.8, RADIUS_MAX * 0.8) * tv(T_DROP),
                floor_y: hip[1] - HIP_HEIGHT,
                landed: false,
                fresh: false,
                age: 0.0,
                group: 0,
                skip_until: 0.0,
                exact: false,
                mapped: false,
                tested_from: [f32::NAN; 3],
                band: 0.0,
                grow: 0.0,
                shape: NO_SHAPE,
                seq: 0,
            });
        }
    }
    sim.drips.retain(|d| d.time_left > 0.0);
    if let Ok(mut c) = CHAR_CACHE.lock() {
        c.clear();
        for d in sim.drips.iter() {
            let h = unsafe { read_hip(d.motile) };
            if h.iter().all(|v| v.is_finite()) {
                c.push(h);
            }
        }
    }
    sim.blobs.extend(new_drops);

    cost_note(8, t_drips.elapsed().as_secs_f64() * 1000.0, 0);
    // Sub-steps keep the fluid forces stable.
    let steps = ((dt * 120.0).ceil() as usize).clamp(1, 8);
    let h = dt / steps as f32;
    let fluid = FLUID_ON.load(Ordering::Relaxed);
    let sticky = tv(T_STICKY);
    let grav = tv(T_GRAVITY);
    let drag = (1.0 - AIR_DRAG * h).max(0.0);

    // Where each flying drop was at the start of this frame (for the collision test).
    let start: Vec<[f32; 3]> = sim.blobs.iter().map(|b| b.pos).collect();
    let have_level = LEVEL.lock().map(|g| g.is_some()).unwrap_or(false);

    // Cast-off ropes: each drop linked to the next one along its rope (same group, next seq).
    let rope_links: Vec<(usize, usize)> = {
        let mut r: Vec<(u32, u32, usize)> = sim
            .blobs
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.landed && b.group & CAST_OFF_GROUP != 0 && b.group != u32::MAX)
            .map(|(i, b)| (b.group, b.seq, i))
            .collect();
        r.sort_unstable();
        r.windows(2).filter(|w| w[0].0 == w[1].0 && w[1].1 == w[0].1 + 1).map(|w| (w[0].2, w[1].2)).collect()
    };
    let rope_stiff = (tv(T_CO_STRING) / 5.0).clamp(0.0, 1.0);

    let t_sim = std::time::Instant::now();
    for _ in 0..steps {
        // --- fluid forces between neighbouring flying particles of the same jet ---
        let co_sticky = tv(T_CO_STRING);
        if fluid {
            // Only drops of the same jet interact: group them by jet, then pairs within a
            // jet only, on compact copies of position / velocity / radius.
            let mut idx: Vec<(u32, usize)> = sim
                .blobs
                .iter()
                .enumerate()
                .filter(|(_, b)| !b.landed && b.group != 0 && b.group != u32::MAX)
                .map(|(i, b)| (b.group, i))
                .collect();
            idx.sort_unstable_by_key(|e| e.0);
            let pos: Vec<[f32; 3]> = idx.iter().map(|e| sim.blobs[e.1].pos).collect();
            let vel: Vec<[f32; 3]> = idx.iter().map(|e| sim.blobs[e.1].vel).collect();
            let rad: Vec<f32> = idx.iter().map(|e| sim.blobs[e.1].radius).collect();
            let mut dvel = vec![[0.0f32; 3]; idx.len()];
            let r2max = COHESION_RANGE * COHESION_RANGE;
            let mut st = 0;
            while st < idx.len() {
                let g = idx[st].0;
                let mut en = st;
                while en < idx.len() && idx[en].0 == g {
                    en += 1;
                }
                if g & CAST_OFF_GROUP != 0 {
                    st = en; // a cast-off rope: held by its length constraints, not springs
                    continue;
                }
                let en_cap = en.min(st + 400); // (a huge jet: its first 400 drops hold together)
                for a in st..en_cap {
                    let pa = pos[a];
                    for c in (a + 1)..en_cap {
                        let dv = [pos[c][0] - pa[0], pos[c][1] - pa[1], pos[c][2] - pa[2]];
                        let d2 = dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2];
                        if d2 >= r2max || d2 < 1e-8 {
                            continue;
                        }
                        let dist = d2.sqrt();
                        let rest = (rad[a] + rad[c]) * 0.9;
                        // Spring: pull together when apart, push apart when overlapping.
                        let f = COHESION_STRENGTH * sticky * (dist - rest) / dist;
                        // Fade the pull out towards the edge of the range so ropes can snap.
                        let fade = 1.0 - dist / COHESION_RANGE;
                        for k in 0..3 {
                            let d = (dv[k] * f * fade + (vel[c][k] - vel[a][k]) * VISCOSITY * sticky * fade) * h * 0.5;
                            dvel[a][k] += d;
                            dvel[c][k] -= d;
                        }
                    }
                }
                st = en;
            }
            for (n, e) in idx.iter().enumerate() {
                let b = &mut sim.blobs[e.1];
                for k in 0..3 {
                    b.vel[k] += dvel[n][k];
                }
            }
        }
        // --- move ---
        for b in sim.blobs.iter_mut() {
            if b.landed {
                continue;
            }
            b.age += h;
            b.vel[1] -= GRAVITY * grav * h;
            for k in 0..3 {
                b.vel[k] *= drag;
                b.pos[k] += b.vel[k] * h;
            }
            // TEST BUILD: no estimated floor. Drops land only where the game's collision says.
            if !have_level && b.pos[1] < b.floor_y - 3000.0 {
                b.landed = true;
                b.radius = 0.0;
            }
        }
        // --- cast-off ropes: pull over-stretched links back, move linked drops together ---
        if rope_stiff > 0.0 {
            let lim = CAST_OFF_SPACING * 1.15;
            for &(ia, ib) in rope_links.iter() {
                let (pa, pb, va, vb) = (sim.blobs[ia].pos, sim.blobs[ib].pos, sim.blobs[ia].vel, sim.blobs[ib].vel);
                let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                let mut na = pa;
                let mut nb = pb;
                if dist > lim {
                    let c = (dist - lim) / dist * 0.5 * rope_stiff;
                    for k in 0..3 {
                        na[k] += d[k] * c;
                        nb[k] -= d[k] * c;
                    }
                }
                let mut nva = va;
                let mut nvb = vb;
                for k in 0..3 {
                    let avg = (va[k] + vb[k]) * 0.5;
                    nva[k] += (avg - va[k]) * rope_stiff * 0.5;
                    nvb[k] += (avg - vb[k]) * rope_stiff * 0.5;
                }
                sim.blobs[ia].pos = na;
                sim.blobs[ib].pos = nb;
                sim.blobs[ia].vel = nva;
                sim.blobs[ib].vel = nvb;
            }
        }
    }

    cost_note(5, t_sim.elapsed().as_secs_f64() * 1000.0, 0);

    // --- collide each flying drop's path with the game's own geometry ---
    // At most RAY_BUDGET tests per frame, taken in turns; a drop that waits keeps
    // its untested path and is tested along all of it next time (nothing slips through).
    if !have_level && sim.blobs.iter().any(|b| !b.landed) && test_log_ok() {
        error_f!("Cruor TEST: the level's collision isn't read yet - drops can't land");
    }
    if have_level {
        let splat = SPLAT_GROWTH * tv(T_SPLAT);
        let mut walls: Vec<WallSplat> = Vec::new();
        let mut new_runners: Vec<Runner> = Vec::new();
        let t_start = std::time::Instant::now();
        let mut tests = 0usize;
        set_stage(9);
        let t_collide = std::time::Instant::now();
        let bodies = if sim.blobs.iter().any(|b| !b.landed) { collect_bodies() } else { Vec::new() };
        set_stage(2);
        let n = sim.blobs.len();
        let first = if n > 0 { ROUND.fetch_add(RAY_BUDGET as u32, Ordering::Relaxed) as usize % n } else { 0 };
        for step in 0..n {
            let i = (first + step) % n;
            let b = &mut sim.blobs[i];
            if b.landed {
                continue;
            }
            if b.tested_from[0].is_nan() {
                b.tested_from = if i < start.len() { start[i] } else { b.pos };
            }
            if tests >= RAY_BUDGET || (tests & 15 == 0 && t_start.elapsed().as_secs_f32() > COLLISION_BUDGET_S) {
                continue; // its turn comes next frame (its whole path is tested then)
            }
            tests += 1;
            let from = b.tested_from;
            b.tested_from = b.pos;
            // a character's body (the game's own physics parts)?
            if b.age > BODY_GRACE && !bodies.is_empty() {
                let d = sub3(b.pos, from);
                let mut best: Option<(f32, usize)> = None;
                for (k, bp) in bodies.iter().enumerate() {
                    if !seg_hits_sphere(from, d, bp.centre, bp.radius) {
                        continue;
                    }
                    if let Some(t) = segment_tetra(from, d, &bp.v) {
                        if best.map(|x| t < x.0).unwrap_or(true) {
                            best = Some((t, k));
                        }
                    }
                }
                if let Some((t, k)) = best {
                    let at = [from[0] + d[0] * t, from[1] + d[1] * t, from[2] + d[2] * t];
                    let amount = (0.03 + b.radius * 0.02).min(0.15);
                    drop_hit_body(&bodies[k], at, amount);
                    b.landed = true;
                    b.radius = 0.0;
                    b.vel = [0.0; 3];
                    continue;
                }
            }
            let hit = match game_raycast(from, b.pos) {
                Some(h) => h,
                None => {
                    // fell out of the world: forget it
                    if b.pos[1] < b.floor_y - 3000.0 {
                        b.landed = true;
                        b.radius = 0.0;
                    }
                    continue;
                }
            };
            let n = hit.normal;
            // lift the contact point a hair off the surface
            let at = [hit.pos[0] + n[0] * 0.3, hit.pos[1] + n[1] * 0.3, hit.pos[2] + n[2] * 0.3];
            if let Some(prop) = hit.prop {
                // (skip character bodies - animated meshes don't match their collision)
                if near_character(hit.pos) {
                    continue;
                }
                // A loose object: the blood goes on that exact object and moves with it.
                stick_to_prop(prop, hit.pos, n, b.radius * splat * 1.2);
                b.landed = true;
                b.radius = 0.0;
                b.vel = [0.0; 3];
            } else if n[1] > 0.5 {
                // Floor, stair step, ledge, table top of the level...
                // A flat floor keeps the tight band; a sloped collision surface (the
                // game's stairs are a hidden ramp through the visible steps) gets a
                // wider one so the stain shows on the treads around it.
                b.band = if n[1] > 0.95 { 0.0 } else { 16.0 + 45.0 * (1.0 - n[1]) };
                b.pos = at;
                land(b, at);
                b.exact = true;
                COL_FLOOR.fetch_add(1, Ordering::Relaxed);
            } else if n[1] > -0.5 {
                // Wall (or a steep slope): a splat with a drip.
                let rw = b.radius * splat * 1.3;
                // shape: stretched along the drop's path across the wall, spiky when fast
                let along_x = n[2].abs() >= n[0].abs();
                let (tu, tvv) = (if along_x { b.vel[0] } else { b.vel[2] }, b.vel[1]);
                let tang = (tu * tu + tvv * tvv).sqrt();
                let vn = dot3(b.vel, n).abs();
                let spd = dot3(b.vel, b.vel).sqrt();
                let wspike = ((spd - 250.0) / 450.0).clamp(0.0, 1.0) * rand_range(0.6, 1.0);
                let wshape = if tang > 1.0 {
                    [tu / tang, tvv / tang, (1.0 + 1.6 * tang / (vn + 60.0)).min(4.5), if tang / (vn + 60.0) < 0.3 { wspike * 0.5 } else { 0.0 }]
                } else {
                    [1.0, 0.0, 1.0, wspike]
                };
                walls.push(WallSplat { pos: at, normal: n, radius: rw, seed: rand01(), mapped: false, streak: 0.0, shape: wshape });
                if rw >= RUNNER_MIN_SPLAT && rand01() < RUNNER_CHANCE {
                    // a big hit starts several drips side by side across its width
                    let horiz = norm3([-n[2], 0.0, n[0]]);
                    let count = (1 + (rw / 2.5) as usize).min(4);
                    for k in 0..count {
                        let off = if count > 1 { (k as f32 / (count - 1) as f32 - 0.5) * rw * 1.3 + (rand01() - 0.5) * rw * 0.3 } else { 0.0 };
                        let len = rw * rand_range(5.0, 14.0) * if k == count / 2 { 1.0 } else { rand_range(0.4, 1.0) };
                        new_runners.push(Runner {
                            pos: [at[0] + horiz[0] * off, at[1] - rand01() * rw * 0.3, at[2] + horiz[2] * off],
                            normal: n,
                            r0: (rw * if count > 1 { rand_range(0.25, 0.4) } else { 0.45 }).max(0.5),
                            left: len,
                            len,
                            speed: 0.0,
                            max_speed: rand_range(4.0, 11.0),
                            since_deposit: 0.0,
                            since_check: 0.0,
                        });
                    }
                }
                b.landed = true;
                b.radius = 0.0;
                b.vel = [0.0; 3];
                COL_WALL.fetch_add(1, Ordering::Relaxed);
            } else {
                // Underside of something (ceiling, table underneath): stop rising, drop off.
                b.pos = [at[0] + n[0], at[1] + n[1], at[2] + n[2]];
                b.vel = [b.vel[0] * 0.2, -b.vel[1].abs() * 0.1, b.vel[2] * 0.2];
            }
        }
        note_ray_cost(tests, t_start.elapsed());
        sim.blobs.retain(|b| !(b.landed && b.radius <= 0.0));
        if !new_runners.is_empty() {
            let room = MAX_RUNNERS.saturating_sub(sim.runners.len());
            sim.runners.extend(new_runners.into_iter().take(room));
        }
        let t_run = std::time::Instant::now();
        cost_note(6, t_collide.elapsed().as_secs_f64() * 1000.0, 0);
        set_stage(3);
        step_runners(sim, dt);
        set_stage(1);
        cost_note(3, t_run.elapsed().as_secs_f64() * 1000.0, 0);
        if !walls.is_empty() {
            sim.walls.extend(walls);
            if sim.walls.len() > MAX_WALL_SPLATS {
                let extra = sim.walls.len() - MAX_WALL_SPLATS;
                sim.walls.drain(0..extra);
            }
        }
    }

    // Splash droplets from last frame's impacts become stains now.
    if let Ok(mut sats) = SATELLITES.lock() {
        sim.blobs.extend(sats.drain(..));
    }
    // Merge freshly landed splats into touching puddles (area is conserved).
    let t_pool = std::time::Instant::now();
    set_stage(4);
    // (puddles are found through a grid of 64 cm cells instead of scanning them all)
    let fresh: Vec<usize> = sim.blobs.iter().enumerate().filter(|(_, b)| b.landed && b.fresh).map(|(i, _)| i).collect();
    if !fresh.is_empty() {
        const CELL: f32 = 64.0;
        let key = |x: f32, z: f32| ((x / CELL).floor() as i32, (z / CELL).floor() as i32);
        let mut grid: std::collections::HashMap<(i32, i32), Vec<u32>> = std::collections::HashMap::new();
        for (j, b) in sim.blobs.iter().enumerate() {
            if b.landed && !b.fresh && b.radius > 0.0 {
                grid.entry(key(b.pos[0], b.pos[2])).or_default().push(j as u32);
            }
        }
        let mut remove = vec![false; sim.blobs.len()];
        let mut any_removed = false;
        for &i in fresh.iter() {
            sim.blobs[i].fresh = false;
            let a = sim.blobs[i];
            let reach = ((a.radius + POOL_MAX_RADIUS) * 0.6 / CELL).ceil() as i32;
            let (cx, cz) = key(a.pos[0], a.pos[2]);
            let mut target: Option<usize> = None;
            'search: for gx in (cx - reach)..=(cx + reach) {
                for gz in (cz - reach)..=(cz + reach) {
                    if let Some(list) = grid.get(&(gx, gz)) {
                        for &j in list.iter() {
                            let j = j as usize;
                            if j == i || remove[j] {
                                continue;
                            }
                            let b = &sim.blobs[j];
                            let dx = a.pos[0] - b.pos[0];
                            let dz = a.pos[2] - b.pos[2];
                            // a streak keeps its shape: it only pools when it lands
                            // almost on top of another splat
                            let reach_f = if a.shape[2] > 1.5 || b.shape[2] > 1.5 { 0.25 } else { 0.6 };
                            let rr = (a.radius + b.radius) * reach_f;
                            if dx * dx + dz * dz < rr * rr && (a.pos[1] - b.pos[1]).abs() < 4.0 {
                                target = Some(j);
                                break 'search;
                            }
                        }
                    }
                }
            }
            match target {
                Some(j) => {
                    // the pool takes the new blood in and spreads out over a few seconds
                    // (area kept, counting what it's still spreading)
                    let b = sim.blobs[j];
                    let rb = b.radius + b.grow;
                    let area = a.radius * a.radius + rb * rb;
                    let w = (rb * rb) / area;
                    let m = &mut sim.blobs[j];
                    m.pos[0] = b.pos[0] * w + a.pos[0] * (1.0 - w);
                    m.pos[2] = b.pos[2] * w + a.pos[2] * (1.0 - w);
                    m.grow = (area.sqrt().min(POOL_MAX_RADIUS) - m.radius).max(0.0);
                    m.mapped = false;
                    // a pool rounds off as it takes blood in (by how much of it is new)
                    m.shape = [m.shape[0], m.shape[1], 1.0 + (m.shape[2] - 1.0) * w, m.shape[3] * w];
                    remove[i] = true;
                    any_removed = true;
                }
                None => {
                    // stays: later fresh drops this frame can join it
                    grid.entry(key(a.pos[0], a.pos[2])).or_default().push(i as u32);
                }
            }
        }
        if any_removed {
            let mut k = 0;
            sim.blobs.retain(|_| {
                let keep = !remove[k];
                k += 1;
                keep
            });
        }
    }
    spread_pools(sim, dt);
    cost_note(4, t_pool.elapsed().as_secs_f64() * 1000.0, 0);
    set_stage(0);
}

/// Pools spread out towards the size their blood covers; repainted a few times a second
/// while they grow (painting keeps the larger value, so a bigger repaint just extends it).
fn spread_pools(sim: &mut Sim, dt: f32) {
    static FRAME: AtomicU32 = AtomicU32::new(0);
    let repaint = FRAME.fetch_add(1, Ordering::Relaxed) % 6 == 0;
    let k = (dt * POOL_SPREAD_RATE).min(1.0);
    for b in sim.blobs.iter_mut() {
        if b.landed && b.grow > 0.02 {
            let d = b.grow * k + 0.02;
            let d = d.min(b.grow);
            b.radius += d;
            b.grow -= d;
            if repaint || b.grow <= 0.02 {
                b.mapped = false;
            }
        }
    }
}

/// The stretch each runner moved this frame, painted straight into the wall maps (not
/// stored: the stored 3 cm segments cover it for refills) so streaks grow smoothly.
static RUNNER_TIPS: Mutex<Vec<WallSplat>> = Mutex::new(Vec::new());

/// Runners slide down their wall, leaving a thinning streak.
fn step_runners(sim: &mut Sim, dt: f32) {
    if sim.runners.is_empty() {
        return;
    }
    let mut new_walls: Vec<WallSplat> = Vec::new();
    let mut new_blobs: Vec<Blob> = Vec::new();
    let pool_at = |at: [f32; 3], r: f32| Blob {
        pos: at,
        vel: [0.0; 3],
        radius: r,
        floor_y: at[1],
        landed: true,
        fresh: true,
        age: 0.0,
        group: u32::MAX,
        skip_until: 0.0,
        exact: true,
        mapped: false,
        tested_from: [f32::NAN; 3],
        band: 0.0,
        grow: 0.0,
        shape: NO_SHAPE,
        seq: 0,
    };
    for r in sim.runners.iter_mut() {
        // downhill along the wall
        let gn = -r.normal[1];
        let mut d = [-r.normal[0] * gn, -1.0 - r.normal[1] * gn, -r.normal[2] * gn];
        let dl = dot3(d, d).sqrt();
        if dl < 0.25 {
            r.left = 0.0;
            continue;
        }
        d = [d[0] / dl, d[1] / dl, d[2] / dl];
        r.speed = (r.speed + RUNNER_ACCEL * dt).min(r.max_speed);
        let step = (r.speed * dt).min(r.left.max(0.0));
        let before = r.pos;
        r.pos = [r.pos[0] + d[0] * step, r.pos[1] + d[1] * step, r.pos[2] + d[2] * step];
        if let Ok(mut tips) = RUNNER_TIPS.lock() {
            if tips.len() < 2000 {
                let frac_now = (r.left / r.len.max(0.1)).clamp(0.0, 1.0);
                let wob = 1.0 + 0.18 * ((r.len - r.left) * 0.45 + r.r0 * 7.0).sin() + 0.08 * ((r.len - r.left) * 1.3 + r.len).sin();
                let w = (r.r0 * (0.35 + 0.65 * frac_now.sqrt()) * wob).max(0.6);
                tips.push(WallSplat { pos: before, normal: r.normal, radius: w, seed: 0.5, mapped: false, streak: (before[1] - r.pos[1]).max(0.0), shape: NO_SHAPE });
            }
        }
        r.left -= step;
        r.since_deposit += step;
        r.since_check += step;
        let frac = (r.left / r.len.max(0.1)).clamp(0.0, 1.0);
        if r.since_deposit >= 3.0 || r.left <= 0.0 {
            // one segment covering the stretch it ran since the last one
            let seg = r.since_deposit;
            r.since_deposit = 0.0;
            let wob = 1.0 + 0.18 * ((r.len - r.left) * 0.45 + r.r0 * 7.0).sin() + 0.08 * ((r.len - r.left) * 1.3 + r.len).sin();
            let w = (r.r0 * (0.35 + 0.65 * frac.sqrt()) * wob).max(0.6);
            let top = [r.pos[0] - d[0] * seg, r.pos[1] - d[1] * seg, r.pos[2] - d[2] * seg];
            new_walls.push(WallSplat { pos: top, normal: r.normal, radius: w, seed: rand01(), mapped: false, streak: seg * (-d[1]).max(0.0), shape: NO_SHAPE });
        }
        if r.since_check >= 4.0 {
            r.since_check = 0.0;
            // reached the floor?
            let p = [r.pos[0] + r.normal[0] * 1.5, r.pos[1] + r.normal[1] * 1.5, r.pos[2] + r.normal[2] * 1.5];
            if let Some(h) = game_raycast(p, [p[0], p[1] - 4.0, p[2]]) {
                if h.normal[1] > 0.5 && h.prop.is_none() {
                    new_blobs.push(pool_at(h.pos, (r.r0 * 1.6 * (0.4 + frac)).max(1.0)));
                    r.left = 0.0;
                    continue;
                }
            }
            // still on the wall?
            let off = [r.normal[0] * 2.5, r.normal[1] * 2.5, r.normal[2] * 2.5];
            let a = [r.pos[0] + off[0], r.pos[1] + off[1], r.pos[2] + off[2]];
            let b = [r.pos[0] - off[0], r.pos[1] - off[1], r.pos[2] - off[2]];
            match game_raycast(a, b) {
                Some(h) if h.normal[1].abs() < 0.8 && h.prop.is_none() => {
                    r.pos = h.pos;
                    r.normal = h.normal;
                }
                _ => {
                    // ran off the wall's edge: falls as a drop
                    let fall_r = (r.r0 * (0.4 + frac) / (SPLAT_GROWTH * tv(T_SPLAT)).max(0.1)).max(0.3);
                    let mut fb = pool_at([r.pos[0] + r.normal[0] * 1.0, r.pos[1], r.pos[2] + r.normal[2] * 1.0], fall_r);
                    fb.landed = false;
                    fb.fresh = false;
                    fb.exact = false;
                    fb.vel = [0.0, -20.0, 0.0];
                    fb.floor_y = r.pos[1] - 3000.0;
                    fb.group = 0;
                    new_blobs.push(fb);
                    r.left = 0.0;
                    continue;
                }
            }
        }
        if r.left <= 0.0 {
            // the bead stops: a round bulb where it ends, drawn out downward a little
            new_walls.push(WallSplat { pos: r.pos, normal: r.normal, radius: (r.r0 * 0.85).max(0.6), seed: rand01(), mapped: false, streak: 0.0, shape: [0.0, -1.0, 1.35, 0.0] });
        }
    }
    sim.runners.retain(|r| r.left > 0.0);
    if !new_walls.is_empty() {
        sim.walls.extend(new_walls);
        if sim.walls.len() > MAX_WALL_SPLATS {
            let extra = sim.walls.len() - MAX_WALL_SPLATS;
            sim.walls.drain(0..extra);
        }
    }
    sim.blobs.extend(new_blobs);
}


// ============================================================
// Matrix helpers (column-major, like OpenGL)
// ============================================================
type Mat4 = [f32; 16];

fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [0.0f32; 16];
    for c in 0..4 {
        for row in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k * 4 + row] * b[c * 4 + k];
            }
            r[c * 4 + row] = s;
        }
    }
    r
}

fn mat_transpose(m: &Mat4) -> Mat4 {
    let mut r = [0.0f32; 16];
    for c in 0..4 {
        for row in 0..4 {
            r[c * 4 + row] = m[row * 4 + c];
        }
    }
    r
}

fn mat_inverse(m: &Mat4) -> Option<Mat4> {
    let mut inv = [0.0f32; 16];
    inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15]
        + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10];
    inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15]
        - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10];
    inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15]
        + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
    inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14]
        - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
    inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15]
        - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
    inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15]
        + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
    inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15]
        - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
    inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14]
        + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
    inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15]
        + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
    inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15]
        - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
    inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15]
        + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
    inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14]
        - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
    inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11]
        - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
    inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11]
        + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
    inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11]
        - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
    inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10]
        + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];
    let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    for v in inv.iter_mut() {
        *v *= d;
    }
    Some(inv)
}

fn mat_mul_vec(m: &Mat4, v: [f32; 4]) -> [f32; 4] {
    let mut r = [0.0f32; 4];
    for row in 0..4 {
        r[row] = m[row] * v[0] + m[4 + row] * v[1] + m[8 + row] * v[2] + m[12 + row] * v[3];
    }
    r
}


// ============================================================
// Game collision function (only used to learn where the level lives)
// ============================================================
const INTERSECT_SIG: &str = "55 48 89 E5 48 8D A4 24 A0 FE FF FF 48 89 9D C0 FE FF FF 48 89 BD C8 FE FF FF 48 89 B5 D0 FE FF FF 4C 89 A5 D8 FE FF FF 48 89 4D F8";
static WORLD_INTERSECT: AtomicUsize = AtomicUsize::new(0);
static SCENE: AtomicUsize = AtomicUsize::new(0);

/// Find a byte pattern ("??" = any byte) inside the game's exe in memory.
unsafe fn find_pattern(sig: &str) -> Option<usize> {
    let pat: Vec<Option<u8>> = sig
        .split_whitespace()
        .map(|t| if t == "??" { None } else { u8::from_str_radix(t, 16).ok() })
        .collect();
    let base = unsafe { GetModuleHandleA(std::ptr::null()) } as usize;
    if base == 0 {
        return None;
    }
    let e_lfanew = unsafe { ((base + 0x3C) as *const u32).read_unaligned() } as usize;
    let size = unsafe { ((base + e_lfanew + 0x18 + 0x38) as *const u32).read_unaligned() } as usize;
    let mem = unsafe { std::slice::from_raw_parts(base as *const u8, size) };
    'outer: for i in 0..size.saturating_sub(pat.len()) {
        for (j, b) in pat.iter().enumerate() {
            if let Some(b) = b {
                if mem[i + j] != *b {
                    continue 'outer;
                }
            }
        }
        return Some(base + i);
    }
    None
}

// ============================================================
// Camera capture: the game uploads ObjMat[0] (object->world) and ObjMat[1]
// (object->screen) for every object; screen-from-world = ObjMat[1] * inverse(ObjMat[0]).
// ============================================================
struct CamCapture {
    objmat_locs: std::collections::HashSet<(u32, i32)>,
    main_programs: std::collections::HashSet<u32>,
    latest: Option<[f32; 32]>,
    samples: u64,
}

static CAM: Mutex<Option<CamCapture>> = Mutex::new(None);
static CURRENT_PROGRAM: AtomicU32 = AtomicU32::new(0);

static REAL_GET_UNIFORM_LOCATION: AtomicUsize = AtomicUsize::new(0);
static REAL_USE_PROGRAM: AtomicUsize = AtomicUsize::new(0);
static REAL_UNIFORM_MATRIX4FV: AtomicUsize = AtomicUsize::new(0);

type GetUniformLocationFn = unsafe extern "system" fn(u32, *const c_char) -> i32;
type UseProgramFn = unsafe extern "system" fn(u32);
type UniformMatrix4fvFn = unsafe extern "system" fn(i32, i32, u8, *const f32);

unsafe extern "system" fn my_get_uniform_location(program: u32, name: *const c_char) -> i32 {
    let real: GetUniformLocationFn =
        unsafe { std::mem::transmute(REAL_GET_UNIFORM_LOCATION.load(Ordering::Relaxed)) };
    let loc = unsafe { real(program, name) };
    if !name.is_null() && loc >= 0 {
        let n = unsafe { CStr::from_ptr(name) }.to_bytes();
        let is_objmat = n.starts_with(b"ObjMat");
        let is_main = n == b"VXGIPos";
        // Remember which uniforms are textures, so we only watch real texture-slot assignments.
        let looks_like_texture = n.ends_with(b"Tex") || n.ends_with(b"Map") || n == b"VoxelGI" || n == b"FrmBuf";
        if looks_like_texture {
            if let Ok(mut g) = SAMPLER_LOCS.lock() {
                g.get_or_insert_with(Default::default).insert((program, loc));
            }
        }
        if is_objmat || is_main {
            if let Ok(mut guard) = CAM.lock() {
                let cam = guard.get_or_insert_with(|| CamCapture {
                    objmat_locs: Default::default(),
                    main_programs: Default::default(),
                    latest: None,
                    samples: 0,
                });
                if is_objmat {
                    cam.objmat_locs.insert((program, loc));
                }
                if is_main {
                    cam.main_programs.insert(program);
                }
            }
        }
    }
    loc
}

unsafe extern "system" fn my_use_program(program: u32) {
    install_swap_hook();
    CURRENT_PROGRAM.store(program, Ordering::Relaxed);
    note_program(program);
    let real: UseProgramFn = unsafe { std::mem::transmute(REAL_USE_PROGRAM.load(Ordering::Relaxed)) };
    unsafe { real(program) };
    feed_stain_uniforms(program);
}

unsafe extern "system" fn my_uniform_matrix4fv(loc: i32, count: i32, transpose: u8, value: *const f32) {
    let t_hook = std::time::Instant::now();
    PERF_HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
    let mut tile_cam: Option<[f32; 32]> = None;
    if count >= 2 && loc >= 0 && !value.is_null() {
        let prog = CURRENT_PROGRAM.load(Ordering::Relaxed);
        if let Ok(mut guard) = CAM.try_lock() {
            if let Some(cam) = guard.as_mut() {
                if cam.main_programs.contains(&prog) && cam.objmat_locs.contains(&(prog, loc)) {
                    let mut m = [0.0f32; 32];
                    unsafe { std::ptr::copy_nonoverlapping(value, m.as_mut_ptr(), 32) };
                    if transpose != 0 {
                        let mut a: Mat4 = [0.0; 16];
                        let mut b: Mat4 = [0.0; 16];
                        a.copy_from_slice(&m[0..16]);
                        b.copy_from_slice(&m[16..32]);
                        m[0..16].copy_from_slice(&mat_transpose(&a));
                        m[16..32].copy_from_slice(&mat_transpose(&b));
                    }
                    cam.latest = Some(m);
                    cam.samples += 1;
                    tile_cam = Some(m);
                }
            }
        }
        if let Some(m) = tile_cam {
            let vp = tile_view_proj(&m);
            with_fs(|fs| {
                let v = fs.cur_viewport;
                // count this draw's camera for this copy of the scene
                let best = match vp {
                    Some(vp) => {
                        let votes = match fs.tile_votes.iter_mut().position(|(tv, _)| *tv == v) {
                            Some(i) => &mut fs.tile_votes[i].1,
                            None => {
                                if fs.tile_votes.len() >= 16 {
                                    return;
                                }
                                fs.tile_votes.push((v, Vec::new()));
                                &mut fs.tile_votes.last_mut().unwrap().1
                            }
                        };
                        match votes.iter_mut().find(|(c, _, _)| same_camera(c, &vp)) {
                            Some(e) => {
                                e.1 = m;
                                e.2 += 1;
                            }
                            None => {
                                if votes.len() < 8 {
                                    votes.push((vp, m, 1));
                                }
                            }
                        }
                        votes.iter().max_by_key(|e| e.2).map(|e| e.1).unwrap_or(m)
                    }
                    None => m,
                };
                match fs.tiles.iter_mut().find(|(tv, _)| *tv == v) {
                    Some(t) => t.1 = best,
                    None => {
                        if fs.tiles.len() < 16 {
                            fs.tiles.push((v, best));
                        }
                    }
                }
            });
        }
    }
    perf_add(&PERF_HOOK_NS, t_hook);
    let real: UniformMatrix4fvFn =
        unsafe { std::mem::transmute(REAL_UNIFORM_MATRIX4FV.load(Ordering::Relaxed)) };
    unsafe { real(loc, count, transpose, value) };
    // Level-only mode: decide per object. The level is drawn unrotated and unscaled;
    // movable objects and characters carry their own rotation.
    let prog_now = CURRENT_PROGRAM.load(Ordering::Relaxed);
    let is_objmat_upload = CAM
        .try_lock()
        .ok()
        .map(|g| g.as_ref().map(|c| c.objmat_locs.contains(&(prog_now, loc))).unwrap_or(false))
        .unwrap_or(false);
    if is_objmat_upload && count >= 2 && !value.is_null() && INJECTED_SHADERS.load(Ordering::Relaxed) > 0 {
        let mut m = [0.0f32; 16];
        unsafe { std::ptr::copy_nonoverlapping(value, m.as_mut_ptr(), 16) };
        if transpose != 0 {
            m = mat_transpose(&m);
        }
        let movable = !level_like(&m);
        if let Ok(mut f) = FRAME_OBJECTS.lock() {
            // (the same object drawn in several passes only needs recording once)
            if f.len() < 6000 && !f.iter().rev().take(8).any(|(o, _)| same_object_this_frame(o, &m)) {
                f.push((m, movable));
            }
        }
        if let Ok(mut p) = PENDING_OBJ.lock() {
            *p = Some((prog_now, m, movable));
        }
    }
    if STAIN_MODE.load(Ordering::Relaxed) == 0 && is_objmat_upload && count >= 2 && !value.is_null() && CURRENT_STAIN_ALLOWED.load(Ordering::Relaxed) {
        let on_loc = CURRENT_ON_LOC.load(Ordering::Relaxed);
        if on_loc >= 0 {
            let m = unsafe { std::slice::from_raw_parts(value, 16) };
            // Level pieces are placed upright and square to the world (turned by whole
            // quarter-turns at most, maybe mirrored or stretched). Characters and loose
            // objects are tilted/turned by arbitrary amounts.
            let e = 0.001f32;
            let upright = m[1].abs() < e && m[4].abs() < e && m[6].abs() < e && m[9].abs() < e && m[5] > e;
            let square = (m[0].abs() < e || m[2].abs() < e) && (m[8].abs() < e || m[10].abs() < e);
            // Square to the world = level. Turned at an angle (rotated stairs, angled
            // floors) = level only if it's placed exactly where a fixed level piece is;
            // characters and props never are.
            let level = upright && (square || is_static_piece(m));
            if let Ok(g) = STAIN_GL.lock() {
                if let Some(Some(sg)) = g.as_ref() {
                    unsafe { (sg.uniform1f)(on_loc, if level { 1.0 } else { 0.0 }) };
                }
            }
        }
    }
}

fn current_view_proj() -> Option<Mat4> {
    let guard = CAM.lock().ok()?;
    let m = guard.as_ref()?.latest?;
    let mut obj_to_world: Mat4 = [0.0; 16];
    let mut obj_to_clip: Mat4 = [0.0; 16];
    obj_to_world.copy_from_slice(&m[0..16]);
    obj_to_clip.copy_from_slice(&m[16..32]);
    let world_to_obj = mat_inverse(&obj_to_world)?;
    Some(mat_mul(&obj_to_clip, &world_to_obj))
}

// ============================================================
// Which hidden image holds the game's scene (so we can copy its depth)
// ============================================================
struct FrameState {
    session_main: u32, // main-shader switches since the current framebuffer was bound
    /// Areas of the current framebuffer the scene was drawn into this session, each with
    /// the camera used for it (supersampling draws several nudged copies side by side).
    tiles: Vec<([i32; 4], [f32; 32])>,
    /// Per copy of the scene: the different cameras the game drew it with this visit,
    /// each with how many draws used it. The copy's camera (in `tiles`) is the one most
    /// draws used - the real scene camera, not whatever happened to be drawn last.
    tile_votes: Vec<([i32; 4], Vec<(Mat4, [f32; 32], u32)>)>,
    /// Blood already drawn into the current visit (at the game's snapshot of it).
    drawn_session: bool,
    cur_fbo: u32,
    cur_viewport: [i32; 4],
    counts: std::collections::HashMap<u32, u32>,
    scene_fbo: Option<u32>,
    viewports: Vec<[i32; 4]>,
    scene_vp: Option<[i32; 4]>,
}

static FS: Mutex<Option<FrameState>> = Mutex::new(None);
static REAL_BIND_FRAMEBUFFER: AtomicUsize = AtomicUsize::new(0);
static REAL_VIEWPORT: AtomicUsize = AtomicUsize::new(0);
type BindFramebufferFn = unsafe extern "system" fn(u32, u32);
type ViewportFn = unsafe extern "system" fn(i32, i32, i32, i32);

fn with_fs<R>(f: impl FnOnce(&mut FrameState) -> R) -> Option<R> {
    let mut guard = FS.lock().ok()?;
    let fs = guard.get_or_insert_with(|| FrameState {
        session_main: 0,
        tiles: Vec::new(),
        tile_votes: Vec::new(),
        drawn_session: false,
        cur_fbo: 0,
        cur_viewport: [0; 4],
        counts: Default::default(),
        scene_fbo: None,
        viewports: Vec::new(),
        scene_vp: None,
    });
    Some(f(fs))
}

fn is_main_program(program: u32) -> bool {
    CAM.lock()
        .ok()
        .and_then(|g| g.as_ref().map(|c| c.main_programs.contains(&program)))
        .unwrap_or(false)
}

fn note_program(program: u32) {
    if !is_main_program(program) {
        return;
    }
    with_fs(|fs| {
        fs.session_main += 1;
        *fs.counts.entry(fs.cur_fbo).or_insert(0) += 1;
        if fs.scene_fbo == Some(fs.cur_fbo) && fs.viewports.len() < 8 && !fs.viewports.contains(&fs.cur_viewport) {
            let v = fs.cur_viewport;
            fs.viewports.push(v);
        }
    });
}

unsafe extern "system" fn my_viewport(x: i32, y: i32, w: i32, h: i32) {
    visit_with(|v| {
        let a = [x, y, w, h];
        if !v.areas.contains(&a) && v.areas.len() < 12 {
            v.areas.push(a);
        }
    });
    with_fs(|fs| fs.cur_viewport = [x, y, w, h]);
    let real: ViewportFn = unsafe { std::mem::transmute(REAL_VIEWPORT.load(Ordering::Relaxed)) };
    unsafe { real(x, y, w, h) }
}

unsafe extern "system" fn my_bind_framebuffer(target: u32, fbo: u32) {
    if target == GL_FRAMEBUFFER || target == GL_READ_FRAMEBUFFER {
        CUR_READ_FBO.store(fbo, Ordering::Relaxed);
    }
    if target == GL_FRAMEBUFFER || target == GL_DRAW_FRAMEBUFFER {
        let old = CUR_DRAW_FBO.swap(fbo, Ordering::Relaxed);
        if old != fbo {
            let done = SNAP.lock().ok().and_then(|mut sn| if sn.as_ref().map(|s| s.fbo == old).unwrap_or(false) { sn.take() } else { None });
            if let Some(s) = done {
                unsafe { draw_into_combined(s) };
            }
        }
        // The game is about to leave its scene buffer: the scene is finished (and still
        // bound, with its own depth). Draw the flying blood into it now, with the camera
        // the scene was just drawn with - before the game's post-processing and UI.
        let leaving = with_fs(|fs| {
            // (if the blood already went in at the game's snapshot, don't draw it again)
            // (blood is drawn only at the game's combine / copy - see before_snapshot)
            let l = false;
            let votes = std::mem::take(&mut fs.tile_votes);
            report_cameras(&votes);
            let out = (l, fs.cur_fbo, std::mem::take(&mut fs.tiles));
            fs.cur_fbo = fbo;
            fs.session_main = 0;
            fs.drawn_session = false;
            out
        });
        if let Some((true, scene, tiles)) = leaving {
            if TRACE_ACTIVE.load(Ordering::Relaxed) {
                trace_flush_draws();
                trace(format!(
                    "   ** BLOOD DRAWN HERE into buffer {} (after the visit above) - copies: {}",
                    scene,
                    tiles.iter().map(|(v, _)| format!("{},{} {}x{}", v[0], v[1], v[2], v[3])).collect::<Vec<_>>().join(" | ")
                ));
            }
            unsafe { in_scene_draw(scene, &tiles) };
        }
        if TRACE_ACTIVE.load(Ordering::Relaxed) {
            trace_flush_draws();
            if let Ok(mut v) = VISIT.lock() {
                *v = None;
            }
            visit_start(fbo);
        }
    } else if target == GL_READ_FRAMEBUFFER && TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace(format!("   (sets buffer {} as the source for reading/copying)", fbo));
    }
    let real: BindFramebufferFn = unsafe { std::mem::transmute(REAL_BIND_FRAMEBUFFER.load(Ordering::Relaxed)) };
    unsafe { real(target, fbo) };
    if TRACE_ACTIVE.load(Ordering::Relaxed) && (target == GL_FRAMEBUFFER || target == GL_DRAW_FRAMEBUFFER) {
        unsafe { note_fb_textures(fbo) };
    }
}

// ============================================================
// OpenGL access
// ============================================================
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
    fn RtlCaptureStackBackTrace(skip: u32, count: u32, trace: *mut *mut c_void, hash: *mut u32) -> u16;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}
#[link(name = "user32")]
extern "system" {
    fn WindowFromDC(hdc: *mut c_void) -> *mut c_void;
    fn GetClientRect(hwnd: *mut c_void, rect: *mut [i32; 4]) -> i32;
    fn GetAsyncKeyState(vkey: i32) -> i16;
}

unsafe fn gl_resolve(name: &str) -> *mut c_void {
    let cname = std::ffi::CString::new(name).unwrap();
    let ogl = unsafe { GetModuleHandleA(b"opengl32.dll\0".as_ptr() as *const c_char) };
    if ogl.is_null() {
        return std::ptr::null_mut();
    }
    let wgl_get = unsafe { GetProcAddress(ogl, b"wglGetProcAddress\0".as_ptr() as *const c_char) };
    if !wgl_get.is_null() {
        let wgl_get: unsafe extern "system" fn(*const c_char) -> *mut c_void =
            unsafe { std::mem::transmute(wgl_get) };
        let p = unsafe { wgl_get(cname.as_ptr()) };
        let pi = p as isize;
        if !p.is_null() && pi != 1 && pi != 2 && pi != 3 && pi != -1 {
            return p;
        }
    }
    unsafe { GetProcAddress(ogl, cname.as_ptr()) }
}

macro_rules! gl_struct {
    ($($field:ident = $name:literal : fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        struct Gl { $($field: unsafe extern "system" fn($($arg),*) $(-> $ret)?,)* }
        impl Gl {
            unsafe fn load() -> Option<Gl> {
                Some(Gl { $($field: {
                    let p = unsafe { gl_resolve($name) };
                    if p.is_null() {
                        error_f!("Cruor: missing OpenGL function {}", $name);
                        return None;
                    }
                    unsafe { std::mem::transmute::<*mut c_void, unsafe extern "system" fn($($arg),*) $(-> $ret)?>(p) }
                },)* })
            }
        }
    };
}

gl_struct! {
    get_integerv = "glGetIntegerv": fn(u32, *mut i32);
    get_booleanv = "glGetBooleanv": fn(u32, *mut u8);
    is_enabled = "glIsEnabled": fn(u32) -> u8;
    enable = "glEnable": fn(u32);
    disable = "glDisable": fn(u32);
    blend_func_separate = "glBlendFuncSeparate": fn(u32, u32, u32, u32);
    depth_mask = "glDepthMask": fn(u8);
    depth_func = "glDepthFunc": fn(u32);
    color_mask = "glColorMask": fn(u8, u8, u8, u8);
    viewport = "glViewport": fn(i32, i32, i32, i32);
    use_program = "glUseProgram": fn(u32);
    gen_vertex_arrays = "glGenVertexArrays": fn(i32, *mut u32);
    bind_vertex_array = "glBindVertexArray": fn(u32);
    gen_buffers = "glGenBuffers": fn(i32, *mut u32);
    bind_buffer = "glBindBuffer": fn(u32, u32);
    buffer_data = "glBufferData": fn(u32, isize, *const c_void, u32);
    vertex_attrib_pointer = "glVertexAttribPointer": fn(u32, i32, u32, u8, i32, *const c_void);
    enable_vertex_attrib_array = "glEnableVertexAttribArray": fn(u32);
    create_shader = "glCreateShader": fn(u32) -> u32;
    shader_source = "glShaderSource": fn(u32, i32, *const *const c_char, *const i32);
    compile_shader = "glCompileShader": fn(u32);
    get_shaderiv = "glGetShaderiv": fn(u32, u32, *mut i32);
    get_shader_info_log = "glGetShaderInfoLog": fn(u32, i32, *mut i32, *mut c_char);
    create_program = "glCreateProgram": fn() -> u32;
    attach_shader = "glAttachShader": fn(u32, u32);
    bind_attrib_location = "glBindAttribLocation": fn(u32, u32, *const c_char);
    link_program = "glLinkProgram": fn(u32);
    get_programiv = "glGetProgramiv": fn(u32, u32, *mut i32);
    get_program_info_log = "glGetProgramInfoLog": fn(u32, i32, *mut i32, *mut c_char);
    get_uniform_location = "glGetUniformLocation": fn(u32, *const c_char) -> i32;
    uniform_matrix4fv = "glUniformMatrix4fv": fn(i32, i32, u8, *const f32);
    uniform1f = "glUniform1f": fn(i32, f32);
    uniform1i = "glUniform1i": fn(i32, i32);
    uniform2f = "glUniform2f": fn(i32, f32, f32);
    uniform3f = "glUniform3f": fn(i32, f32, f32, f32);
    draw_arrays = "glDrawArrays": fn(u32, i32, i32);
    get_error = "glGetError": fn() -> u32;
    bind_framebuffer = "glBindFramebuffer": fn(u32, u32);
    blit_framebuffer = "glBlitFramebuffer": fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32);
    read_pixels = "glReadPixels": fn(i32, i32, i32, i32, u32, u32, *mut c_void);
    draw_buffer = "glDrawBuffer": fn(u32);
    read_buffer = "glReadBuffer": fn(u32);
    gen_textures = "glGenTextures": fn(i32, *mut u32);
    bind_texture = "glBindTexture": fn(u32, u32);
    active_texture = "glActiveTexture": fn(u32);
    tex_image_2d = "glTexImage2D": fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    tex_parameteri = "glTexParameteri": fn(u32, u32, i32);
    copy_tex_sub_image_2d = "glCopyTexSubImage2D": fn(u32, i32, i32, i32, i32, i32, i32, i32);
    tex_sub_image_2d = "glTexSubImage2D": fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    gen_framebuffers = "glGenFramebuffers": fn(i32, *mut u32);
    gen_renderbuffers = "glGenRenderbuffers": fn(i32, *mut u32);
    bind_renderbuffer = "glBindRenderbuffer": fn(u32, u32);
    renderbuffer_storage = "glRenderbufferStorage": fn(u32, u32, i32, i32);
    framebuffer_renderbuffer = "glFramebufferRenderbuffer": fn(u32, u32, u32, u32);
    get_renderbuffer_parameteriv = "glGetRenderbufferParameteriv": fn(u32, u32, *mut i32);
    get_framebuffer_attachment_parameteriv = "glGetFramebufferAttachmentParameteriv": fn(u32, u32, u32, *mut i32);
    check_framebuffer_status = "glCheckFramebufferStatus": fn(u32) -> u32;
    framebuffer_texture_2d = "glFramebufferTexture2D": fn(u32, u32, u32, u32, i32);
    blend_equation_separate = "glBlendEquationSeparate": fn(u32, u32);
    clear = "glClear": fn(u32);
    clear_color = "glClearColor": fn(f32, f32, f32, f32);
    scissor = "glScissor": fn(i32, i32, i32, i32);
    get_floatv = "glGetFloatv": fn(u32, *mut f32);
    draw_buffers = "glDrawBuffers": fn(i32, *const u32);
    get_tex_level_parameteriv = "glGetTexLevelParameteriv": fn(u32, i32, u32, *mut i32);
    pixel_storei = "glPixelStorei": fn(u32, i32);
    get_uniform_block_index = "glGetUniformBlockIndex": fn(u32, *const c_char) -> u32;
    get_active_uniform_blockiv = "glGetActiveUniformBlockiv": fn(u32, u32, u32, *mut i32);
    get_integeri_v = "glGetIntegeri_v": fn(u32, u32, *mut i32);
    get_buffer_sub_data = "glGetBufferSubData": fn(u32, isize, isize, *mut c_void);
    gen_queries = "glGenQueries": fn(i32, *mut u32);
    begin_query = "glBeginQuery": fn(u32, u32);
    end_query = "glEndQuery": fn(u32);
    get_query_objectuiv = "glGetQueryObjectuiv": fn(u32, u32, *mut u32);
    get_current_dc = "wglGetCurrentDC": fn() -> *mut c_void;
    get_current_context = "wglGetCurrentContext": fn() -> *mut c_void;
    is_program = "glIsProgram": fn(u32) -> u8;
    is_texture = "glIsTexture": fn(u32) -> u8;
}

const GL_FLOAT: u32 = 0x1406;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_TRIANGLES: u32 = 0x0004;
const GL_BLEND: u32 = 0x0BE2;
const GL_DEPTH_TEST: u32 = 0x0B71;
const GL_CULL_FACE: u32 = 0x0B44;
const GL_SCISSOR_TEST: u32 = 0x0C11;
const GL_STENCIL_TEST: u32 = 0x0B90;
const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_STREAM_DRAW: u32 = 0x88E0;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_CURRENT_PROGRAM: u32 = 0x8B8D;
const GL_VERTEX_ARRAY_BINDING: u32 = 0x85B5;
const GL_ARRAY_BUFFER_BINDING: u32 = 0x8894;
const GL_VIEWPORT: u32 = 0x0BA2;
const GL_DEPTH_WRITEMASK: u32 = 0x0B72;
const GL_DEPTH_FUNC: u32 = 0x0B74;
const GL_LEQUAL: u32 = 0x0203;
const GL_BLEND_SRC_RGB: u32 = 0x80C9;
const GL_BLEND_DST_RGB: u32 = 0x80C8;
const GL_BLEND_SRC_ALPHA: u32 = 0x80CB;
const GL_BLEND_DST_ALPHA: u32 = 0x80CA;
const GL_SRC_ALPHA: u32 = 0x0302;
const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
const GL_ZERO: u32 = 0x0000;
const GL_ONE: u32 = 0x0001;
const GL_COLOR_WRITEMASK: u32 = 0x0C23;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const GL_READ_FRAMEBUFFER_BINDING: u32 = 0x8CAA;
const GL_DRAW_BUFFER: u32 = 0x0C01;
const GL_READ_BUFFER: u32 = 0x0C02;
const GL_BACK: u32 = 0x0405;
const GL_DEPTH_BUFFER_BIT: u32 = 0x0100;
const GL_NEAREST: u32 = 0x2600;
const GL_LINEAR: u32 = 0x2601;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE0: u32 = 0x84C0;
const GL_ACTIVE_TEXTURE: u32 = 0x84E0;
const GL_TEXTURE_BINDING_2D: u32 = 0x8069;
const GL_RGBA: u32 = 0x1908;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
/// Texture unit we borrow for the copy of the screen.
const BG_UNIT: u32 = 7;

// ============================================================
// Liquid shaders. Each particle is drawn as a real 3D drop (per-pixel shape,
// normal and depth). Its colour comes from the scene behind it: the picture on
// screen is copied, and the drop refracts and tints it, so it is lit exactly
// like its surroundings, plus wet highlights and edge reflections.
// ============================================================
const BLOB_VS: &str = "#version 150
in vec4 aBlob;
in vec2 aCorner;
in vec3 aVel;
uniform mat4 uViewProj;
uniform vec3 uRight;
uniform vec3 uFwd;
uniform float uScale;
uniform float uStretch;
out vec2 vC;
out vec3 vCenter;
out vec3 vAxis;
out vec3 vPerp;
out float vR;
out float vS;
out float vLanded;
void main() {
    float landed = aBlob.w < 0.0 ? 1.0 : 0.0;
    // Only flying drops are enlarged for visibility; puddles are drawn at real size.
    float r = abs(aBlob.w) * (landed > 0.5 ? 1.0 : uScale);
    vec3 axis;
    vec3 perp;
    float s = 1.0;
    if (landed > 0.5) {
        axis = vec3(1.0, 0.0, 0.0);
        perp = vec3(0.0, 0.0, 1.0);
    } else {
        vec3 v = aVel - dot(aVel, uFwd) * uFwd;
        float sp = length(v);
        if (sp > 1.0) {
            axis = v / sp;
            s = 1.0 + min(sp * uStretch, 2.0);
        } else {
            axis = uRight;
        }
        perp = normalize(cross(uFwd, axis));
    }
    vC = aCorner;
    vCenter = aBlob.xyz;
    vAxis = axis;
    vPerp = perp;
    vR = r;
    vS = s;
    vLanded = landed;
    vec3 wp = aBlob.xyz + axis * aCorner.x * r * s + perp * aCorner.y * r;
    if (landed > 0.5) wp.y += 0.4;
    gl_Position = uViewProj * vec4(wp, 1.0);
}
";

const BLOB_FS: &str = "#version 150
in vec2 vC;
in vec3 vCenter;
in vec3 vAxis;
in vec3 vPerp;
in float vR;
in float vS;
in float vLanded;
uniform mat4 uViewProj;
uniform vec3 uRight;
uniform vec3 uUp;
uniform vec3 uFwd;
uniform vec3 uEye;
uniform vec2 uViewport;
uniform vec2 uVpOrigin;
uniform sampler2D uBg;
uniform float uHaveBg;
uniform float uSceneSpace;
uniform float uDrop;
uniform float uDecodeBg;
out vec4 FragColor;
void main() {
    vec2 c = vC;
    float seed = fract(sin(dot(vCenter, vec3(12.9898, 78.233, 37.719))) * 43758.5453);
    float rim = 1.0;
    if (vLanded > 0.5) {
        float a = atan(c.y, c.x);
        rim = 0.8 + 0.1 * sin(a * 5.0 + seed * 6.283) + 0.06 * sin(a * 9.0 + seed * 17.0) + 0.04 * sin(a * 15.0 + seed * 29.0);
    }
    float dd = length(c) / rim;
    if (dd > 1.0) discard;
    vec3 N;
    vec3 P;
    if (vLanded > 0.5) {
        // Puddle: a thin liquid lens lying on the surface.
        float hgt = sqrt(max(1.0 - dd * dd, 0.0));
        // A thin film: at most ~0.6 cm thick however big the puddle is.
        P = vCenter + vAxis * c.x * vR + vPerp * c.y * vR + vec3(0.0, 0.4 + 0.25 * hgt, 0.0);
        vec3 slope = (vAxis * c.x + vPerp * c.y) * (dd / max(hgt, 0.2)) * 0.15;
        N = normalize(vec3(0.0, 1.0, 0.0) + slope);
    } else {
        // Drop: a (stretched) sphere facing the camera.
        float z = sqrt(max(1.0 - dd * dd, 0.0));
        P = vCenter + vAxis * c.x * vR * vS + vPerp * c.y * vR - uFwd * z * vR;
        N = normalize(vAxis * (c.x / vS) + vPerp * c.y - uFwd * z);
    }
    vec4 clip = uViewProj * vec4(P, 1.0);
    gl_FragDepth = clamp((clip.z / clip.w) * 0.5 + 0.5, 0.0, 1.0);

    vec2 uv = (gl_FragCoord.xy - uVpOrigin) / uViewport;
    vec2 bend = vec2(dot(N, uRight), dot(N, uUp));
    // What's behind the blood (tinted/refracted through it). Without a copy of the
    // scene the blood simply uses its own dark colour.
    vec3 bg = uHaveBg > 0.5 ? texture(uBg, uv).rgb : vec3(0.06);
    vec3 seen = uHaveBg > 0.5 ? texture(uBg, clamp(uv - bend * 0.02, vec2(0.001), vec2(0.999))).rgb : bg;
    if (uDecodeBg > 0.5) {
        // the scene is a plain (non-sRGB) image: convert what's behind to linear light
        // ourselves, as the graphics card does for sRGB scene images - same maths either way
        bg = mix(bg / 12.92, pow((bg + 0.055) / 1.055, vec3(2.4)), step(0.04045, bg));
        seen = mix(seen / 12.92, pow((seen + 0.055) / 1.055, vec3(2.4)), step(0.04045, seen));
    }

    float lum = dot(bg, vec3(0.3, 0.59, 0.11));
    vec3 V = normalize(uEye - P);
    vec3 L = normalize(vec3(0.25, 1.0, 0.35));
    float fres = 0.03 + 0.55 * pow(1.0 - max(dot(N, V), 0.0), 3.0);
    float spec = pow(max(dot(reflect(-L, N), V), 0.0), 80.0);
    vec3 tint = vLanded > 0.5 ? vec3(0.38, 0.025, 0.02) : vec3(0.5, 0.04, 0.03);
    vec3 body = seen * tint + tint * 0.015;
    vec3 col = body * (1.0 - fres) + vec3(lum * 0.5 + 0.01) * fres + vec3(1.0, 0.9, 0.9) * spec * (0.15 + 1.0 * lum);
    float edge = 1.0 - smoothstep(0.88, 1.0, dd);
    if (uSceneSpace > 0.5) {
        // Inside the scene (before the game's colour grading): only tint/darken what's
        // behind (works whatever colour encoding the game uses), highlights no brighter
        // than the scene around them. uDrop follows the darkness setting.
        float sl = max(lum, dot(seen, vec3(0.3, 0.59, 0.11)));
        col = seen * tint * (1.0 - fres) + seen * 0.35 * fres + vec3(1.0, 0.9, 0.9) * spec * sl * 0.6;
        col *= uDrop;
    }
    FragColor = vec4(col, edge);
}
";


struct Renderer {
    gl: Gl,
    program: u32,
    vao: u32,
    vbo: u32,
    bg_tex: u32,
    bg_size: (i32, i32),
    stain_tex: u32,
    wallx_tex: u32,
    wallz_tex: u32,
    atlas_tex: u32,
    // Two stored copies of the game's scene depth (this frame's and last frame's).
    dstore: [u32; 2],
    drb: [u32; 2],
    dsize: (i32, i32),
    dcur: usize,
    dvalid: bool,
    dfail: bool,
    dfbo: u32,
    dretry: u32,
    u_view_proj: i32,
    u_right: i32,
    u_up: i32,
    u_fwd: i32,
    u_eye: i32,
    u_viewport: i32,
    u_vp_origin: i32,
    u_have_bg: i32,
    u_scene_space: i32,
    u_drop: i32,
    u_decode_bg: i32,
    // In-scene background: our own texture + framebuffer in the scene's colour format.
    sbg_tex: u32,
    sbg_fbo: u32,
    sbg_size: (i32, i32),
    sbg_fmt: i32,
    sbg_ok: bool,
    u_scale: i32,
    u_stretch: i32,
    u_bg: i32,
}

struct RendererCell(Option<Renderer>, bool);
/// The OpenGL context our renderer's objects belong to.
static RENDERER_CTX: AtomicUsize = AtomicUsize::new(0);
static RENDERER: Mutex<RendererCell> = Mutex::new(RendererCell(None, false));

unsafe fn compile(gl: &Gl, kind: u32, src: &str) -> Option<u32> {
    unsafe {
        let s = (gl.create_shader)(kind);
        let c = std::ffi::CString::new(src).unwrap();
        let p = c.as_ptr();
        (gl.shader_source)(s, 1, &p, std::ptr::null());
        (gl.compile_shader)(s);
        let mut ok = 0;
        (gl.get_shaderiv)(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut buf = vec![0 as c_char; 4096];
            let mut len = 0;
            (gl.get_shader_info_log)(s, 4096, &mut len, buf.as_mut_ptr());
            error_f!("Cruor shader error: {}", CStr::from_ptr(buf.as_ptr()).to_string_lossy());
            return None;
        }
        Some(s)
    }
}

unsafe fn create_renderer() -> Option<Renderer> {
    unsafe {
        let gl = Gl::load()?;
        let vs = compile(&gl, GL_VERTEX_SHADER, BLOB_VS)?;
        let fs = compile(&gl, GL_FRAGMENT_SHADER, BLOB_FS)?;
        let program = (gl.create_program)();
        (gl.attach_shader)(program, vs);
        (gl.attach_shader)(program, fs);
        (gl.bind_attrib_location)(program, 0, b"aBlob\0".as_ptr() as *const c_char);
        (gl.bind_attrib_location)(program, 1, b"aCorner\0".as_ptr() as *const c_char);
        (gl.bind_attrib_location)(program, 2, b"aVel\0".as_ptr() as *const c_char);
        (gl.link_program)(program);
        let mut ok = 0;
        (gl.get_programiv)(program, GL_LINK_STATUS, &mut ok);
        if ok == 0 {
            let mut buf = vec![0 as c_char; 4096];
            let mut len = 0;
            (gl.get_program_info_log)(program, 4096, &mut len, buf.as_mut_ptr());
            error_f!("Cruor: shader link error: {}", CStr::from_ptr(buf.as_ptr()).to_string_lossy());
            return None;
        }
        let u = |n: &[u8]| unsafe { (gl.get_uniform_location)(program, n.as_ptr() as *const c_char) };
        let u_view_proj = u(b"uViewProj\0");
        let u_right = u(b"uRight\0");
        let u_up = u(b"uUp\0");
        let u_fwd = u(b"uFwd\0");
        let u_eye = u(b"uEye\0");
        let u_viewport = u(b"uViewport\0");
        let u_vp_origin = u(b"uVpOrigin\0");
        let u_have_bg = u(b"uHaveBg\0");
        let u_scene_space = u(b"uSceneSpace\0");
        let u_drop = u(b"uDrop\0");
        let u_decode_bg = u(b"uDecodeBg\0");
        let u_scale = u(b"uScale\0");
        let u_stretch = u(b"uStretch\0");
        let u_bg = u(b"uBg\0");

        let mut prev_vao = 0;
        let mut prev_buf = 0;
        (gl.get_integerv)(GL_VERTEX_ARRAY_BINDING, &mut prev_vao);
        (gl.get_integerv)(GL_ARRAY_BUFFER_BINDING, &mut prev_buf);
        let mut vao = 0;
        let mut vbo = 0;
        (gl.gen_vertex_arrays)(1, &mut vao);
        (gl.gen_buffers)(1, &mut vbo);
        (gl.bind_vertex_array)(vao);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, vbo);
        (gl.enable_vertex_attrib_array)(0);
        (gl.vertex_attrib_pointer)(0, 4, GL_FLOAT, 0, 36, std::ptr::null());
        (gl.enable_vertex_attrib_array)(1);
        (gl.vertex_attrib_pointer)(1, 2, GL_FLOAT, 0, 36, 16usize as *const c_void);
        (gl.enable_vertex_attrib_array)(2);
        (gl.vertex_attrib_pointer)(2, 3, GL_FLOAT, 0, 36, 24usize as *const c_void);
        (gl.bind_vertex_array)(prev_vao as u32);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, prev_buf as u32);

        let mut bg_tex = 0;
        (gl.gen_textures)(1, &mut bg_tex);
        let mut max_units = 16;
        (gl.get_integerv)(0x8872, &mut max_units); // GL_MAX_TEXTURE_IMAGE_UNITS (per shader)
        let base = (max_units.max(16) - 4) as u32;
        UNIT_BASE.store(base, Ordering::Relaxed);
        let mut stain_tex = 0;
        (gl.gen_textures)(1, &mut stain_tex);
        STAIN_TEX_READY.store(false, Ordering::Relaxed);
        gl_verify_again();
        // new, empty textures: the stain maps and the object atlas are sent to them again
        MAP_DIRTY.store(true, Ordering::Relaxed);
        if let Ok(mut g) = ATLAS.lock() {
            if let Some(at) = g.as_mut() {
                at.allocated = false;
                at.dirty.mark_all();
            }
        }
        let mut wallx_tex = 0;
        (gl.gen_textures)(1, &mut wallx_tex);
        let mut wallz_tex = 0;
        (gl.gen_textures)(1, &mut wallz_tex);
        let mut atlas_tex = 0;
        (gl.gen_textures)(1, &mut atlas_tex);

        info_f!("Cruor: liquid renderer ready");
        Some(Renderer {
            gl, program, vao, vbo, bg_tex, bg_size: (0, 0), stain_tex, wallx_tex, wallz_tex, atlas_tex,
            dstore: [0; 2], drb: [0; 2], dsize: (0, 0), dcur: 0, dvalid: false, dfail: false, dfbo: 0, dretry: 0,
            u_view_proj, u_right, u_up, u_fwd, u_eye, u_viewport, u_vp_origin, u_have_bg, u_scene_space, u_drop, u_decode_bg, u_scale, u_stretch, u_bg,
            sbg_tex: 0, sbg_fbo: 0, sbg_size: (0, 0), sbg_fmt: 0, sbg_ok: false,
        })
    }
}

/// 0 = not tried yet, 1 = depth copy works, 2 = depth copy failed.
static DEPTH_COPY_STATE: AtomicU32 = AtomicU32::new(0);
static GL_ERRORS_LOGGED: AtomicU32 = AtomicU32::new(0);

static RESOLVED_LANDINGS: AtomicU32 = AtomicU32::new(0);
static WALL_HITS: AtomicU32 = AtomicU32::new(0);

/// Is this point on or right next to a character we know about (the player, or someone bleeding)?
fn near_character(p: [f32; 3]) -> bool {
    let mut hips: Vec<[f32; 3]> = Vec::new();
    if let Some(h) = player_hips() {
        hips.push(h);
    }
    // (bleeding characters: their drip sources are tracked in the sim, but the sim is
    // locked by our caller, so we use the drip origins cached each frame instead)
    if let Ok(c) = CHAR_CACHE.lock() {
        hips.extend(c.iter().copied());
    }
    hips.iter().any(|h| {
        let dx = p[0] - h[0];
        let dz = p[2] - h[2];
        dx * dx + dz * dz < 45.0 * 45.0 && p[1] > h[1] - 110.0 && p[1] < h[1] + 110.0
    })
}
static CHAR_CACHE: Mutex<Vec<[f32; 3]>> = Mutex::new(Vec::new());

/// Draw the blood on the final picture: copy the scene depth (so things in front
/// hide the blood), land drops on surfaces, copy the picture (so the liquid can
/// refract and take on the scene's lighting), then draw the drops.

// ============================================================
// ON-SCREEN TEXT
// Short messages in the top-left corner (settings changes, hotkeys, warnings) that
// fade after a few seconds. Drawn at the very end of the frame, over everything.
// Font: DejaVu Sans Mono Bold, baked to a 160x120 atlas of 10x20 cells (ASCII 32-126,
// last cell solid for panel backgrounds), 4 bits per pixel.
// ============================================================
const FONT_HEX: &str = "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000e100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007f8000005f808f5000009f17f300000e100001aeb200000006dfc30000007f7000000003f8000007f4000000002f200000000000000000000000000000000000000000000000000ae1000000000000007f8000005f808f500000cc0ae00008dfff5008d1ba0000003fc1380000007f700000000ce1000001ed000000b73f37b000000000000000000000000000000000000000000000002f70000000000000007f8000005f808f500001f90eb0007f6e37400b907d0000005fd0000000007f700000005f900000009f50000039efea3000005f60000000000000000000000000000000000000009e10000000000000007f8000005f808f5002ffffffff00bf4e100008d1ba0000001df6000000007f70000000af400000004fb0000039efe93000005f6000000000000000000000000000000000000002f800000000000000007f800000000000000009f08f2000afdf400001aeb227a4006ffe100000000000000000ef100000001fe00000b73f37b000005f6000000000000000000000000000000000000009e100000000000000007f70000000000000000cc0ae00002dfffc20000389720003fdaf80cd00000000000002fe000000000ef20000002f200008fffffff800000000000000000000000000000000001f8000000000000000005f60000000000000000f90db0000004fefa0029721aeb209f62ef2cd00000000000003fd000000000df300000000000008fffffff80000000000000cfffc00000000000000008f2000000000000000004f400000000000000ffffffff200000e3fe0000008d1ba0bf408fbea00000000000002fe000000000ef200000000000000005f60000000000000000cfffc0000000000000001e90000000000000000000000000000000000009f17f20000600e1fe000000b907d09f701eff600000000000000ef100000001fe000000000000000005f600000009fa00000000000000000bfb0000007f20000000000000000007f8000000000000000cc0ae00000bb2e6f80000008d1ba02ee41bff100000000000000af400000004fb000000000000000005f600000009fa00000000000000000bfb000001ea00000000000000000007f8000000000000001f90ea0000029dfe810000001aeb2003befdcf9000000000000005f900000009f5000000000000000000000000000bf700000000000000000bfb000007f300000000000000000000000000000000000000000000000000e100000000000000000000000000000000000000ce1000001ed0000000000000000000000000000ee10000000000000000000000000ea000000000000000000000000000000000000000000000000000e1000000000000000000000000000000000000003f8000007f40000000000000000000000000003f7000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005dfd5000016dfd000003aeeb4000029eed700000007ff20009fffff500002aeea1000ffffffe00008dfd8000018dfc500000000000000000000000000000000000000000000000000000006cee910004fb1bf4000695fd00000b512df30009611bf8000003fff20009f300000001ee41570000000afd0008f817f80009f61cf40000000000000000000000000000000000000000000000000000049318f9000af504fb000003fd0000000007f900000005fb00000ceff20009f300000008f600000000001ef8000bf302fb000fe005fb00000000000000000000000000004a5000000000005a40000000000006fb000ef302fe000003fd0000000009fb0000001af600008f7ef20009f30000000df200000000005ff20007f818f7002fd004fe00000bfb0000000bfb000000016cff605fffffff605ffd71000000003ef4001ff5e5ff100003fd000000001ef800002ffe600003fd0ef20009feec50000ffaeea2000000afb000009fffa0001fe005ff00000bfb0000000bfb0000039efe93005fffffff60038effa3000001df50001ff5e4ff200003fd000000009fe20000002bf6000df40ef20007513df6001ffc16fb000001ff500007f818f7000bf61cff10000bfb0000000bfb00005ffa500000000000000000004aff600008f800001ff101ff100003fd00000006ff5000000003fd004fa00ef200000004fd000ff600ef100006fe00000ef101fe0002aeebff00000000000000000000005fea4000000000000000000049ef60000bf300000ef202fe000003fd0000003ff70000000001ff004fffffff50000002ff000ef400df30000bf900000fe000df10000001fd000000000000000000000004affe83005fffffff60038dffa400000cf300000af504fb000003fd000001df800000000002fe0000000ef200000004fd000af600ef10002ff300000ef101fe00000005f900000bfb0000000bfb000000017dff505fffffff605ffd710000000000000004fb1af4000003fd00000bf90000001b411bf80000000ef2000a313cf60004fc16fa00007fc0000007f818f80006513de200000bfb0000000bfb00000000004a5000000000005a40000000000cf30000005dfd500009ffffff502ffffffc0004beec600000000ef20003beeb4000005cfd910000cf600000007ded700001aeea2000000bfb0000000cf80000000000000000000000000000000000000cf3000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000fe10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003f800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000efe00001ffffd91000018dfc4000fffea30000cffffff100affffff200029efc4000ff302ff000bfffffc00006ffff6002ff001ef7005fc0000006ff505ff602ff600cf20006dfd7000004beeb200003fff40001ff008fb0000bf8139000ff32cf5000cf60000000af800000001df5139000ff302ff000008f9000000000cf6002ff00bfa0005fc0000006ff909ff602ffb00cf2006fa19f60005f712ad10007fdf80001ff001ff0005fc0000000ff304fd000cf60000000af800000009f80000000ff302ff000008f9000000000cf6002ff08fc10005fc0000006fed0def602fff20cf200df303fe001e80001f5000bf7fc0001ff001ff000af80000000ff300ff200cf60000000af80000000ef40000000ff302ff000008f9000000000cf6002ff5fe200005fc0000006faf4faf602fdf80cf202ff000ff307f11bebf7001ff1ff1001ff007fa000cf60000000ff300ef500cf60000000af80000001ff20000000ff302ff000008f9000000000cf6002ffef8000005fc0000006f7ecf7f602fcad0cf204fe000ef50bb0bd17f7004fc0cf5001fffffb1000df60000000ff300ef500cfffff9000afffffb002ff20000000fffffff000008f9000000000cf6002ffffe100005fc0000006f7bfb7f602fc4f4cf205fe000ef50d91f700f7008f909f9001ff004fd100cf60000000ff300ef500cf60000000af80000001ff21fff400ff302ff000008f9000000000cf6002ff9cf800005fc0000006f77f87f602fc0dacf204fe000ef50e82f500e700cfffffd001ff000cf600af80000000ff300ff200cf60000000af80000000ef4009f400ff302ff000008f9000000000cf6002ff14ff20005fc0000006f70007f602fc08fdf202ff000ff30d81f700f701fe000ef201ff000bf6005fc0000000ff304fd000cf60000000af800000009f8009f400ff302ff000008f9000000000cf5002ff00bf90005fc0000006f70007f602fc02fff200df303fe00bb0bc17f705fb000bf601ff004ff2000bf8129000ff32cf5000cf60000000af800000001ee40af400ff302ff000008f9000039314ff2002ff003ff3005fc0000006f70007f602fc00bff2006fa19f6006f12becf70af80007fa01ffffeb4000018dfc4000fffea30000cffffff100af8000000002aefda200ff302ff000bfffffc0005cefc50002ff000bfb005ffffff706f70007f602fc005ff20006dfd70001da000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003ea313b100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000029dfeb300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000cfffda200006dfd60000ffffd8000007ded81005fffffff603fe000ef407fa000af70ee00000ef07fc000cf70af90009fb00fffffff60000effc0001ea000000000cffe0000000bfc000000000000000cf517fd1006fa19f6000ff21bf90008f71277000008f900003fe000ef403fe000df40cf10000fd00df605fd003ff202ff30000008ff60000ef1000007f30000000000fe0000009fff900000000000000cf500ef400df303fd000ff204fe000ef20000000008f900003fe000ef400ef201ff10af20001fb004fd1df5000af908fb0000003ffe20000ef1000001ea0000000000fe000006fb2bf60000000000000cf500df602ff000ff200ff203ff000ffa1000000008f900003fe000ef400bf504fc009f37f82f9000bfdfb00003ff3ef3000000cff700000ef10000008f2000000000fe00003fa000af3000000000000cf500ef404fe000ef400ff204fe000cffe810000008f900003fe000ef4008f808f8007f5afb3f80002fff300000afefa0000006ffc000000ef10000001e9000000000fe0000000000000000000000000cf517fd105fe000ef500ff21bf70003dfffe4000008f900003fe000ef4004fb0bf5005f6dee4f60000bfc0000002fff3000001eff3000000ef100000008f200000000fe0000000000000000000000000cfffda2004fe000ef400fffff80000006dffd000008f900003fe000ef4001fe0ef1003f8f8f8f40003fff4000000afb0000009ff80000000ef100000002f800000000fe0000000000000000000000000cf50000002ff000ff300ff25fe200000008ff100008f900003fe000ef3000cf4fd0001fcf2fcf2000cfcfc0000008f9000003ffd10000000ef1000000009e10000000fe0000000000000000000000000cf50000000df303fe000ff209fa00000002ff000008f900001ff000ef20009fbf90000efd0cff1005fd0cf6000008f900000cff400000000ef1000000002f80000000fe0000000000000000000000000cf500000006fa19f8000ff202ff300c5217fa000008f900000bf717fb00005fff60000dfa09fe001df404fe100008f900002ffa000000000ef1000000000ae1000000fe0000000000000000000000000cf5000000006dffd1000ff2009fb004adfd81000008f9000002aefea200002fff20000bf705fc008fb000af800008f900002fffffff80000ef10000000002f7000000fe000000000000000000000000000000000000000bf700000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ef10000000000ae100000fe0000000000000000000000000000000000000001b300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000effc000000000000000cffe0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000fffffffff0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002ec000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004fd00000000ef300000000000000000000000000000000000000000000000000000003e80000000000000000ef40000000000000000000003fe0000000000000002cefe0000000000000bf60000000004fd00000000ef30000bf60000005ffff000000000000000000000000000000000000005f4000000000000000ef40000000000000000000003fe0000000000000009f8000000000000000bf60000000004fd00000000ef30000bf6000000003ff00000000000000000000000000000000000000000000000000000000ef40000000000000000000003fe000000000000000bf6000000000000000bf6000000000000000000000000000bf6000000003ff000000000000000000000000000000000000000000000018dfea2000ef9dfb200002aeeb30002bfd9fe00007dfd91000bfffffe0001aedaff000bfacec20006fffd000001ffff30000bf607fe30003ff000006fbecbeb100bfacec200007dfd70000000000000067316fb000efc27fc0002ef5138000cf71cfe0009f815fd00000bf600000af81cff000bfc18fa000004fd00000000ef30000bf66fd200003ff000006f88f87f500bfc19fa0008f918f9000000000000000000ff100ef600ef3008fa0000002fe006fe001fe000cf40000bf600001ff105ff000bf705fd000004fd00000000ef30000bfbfd2000003ff000006f55f65f600bf705fc001ff101ff10000000000004beffff200ef400cf500bf70000004fd004fe004fffffff60000bf600003fe003ff000bf604fd000004fd00000000ef30000bfffc0000003ff000006f55f64f700bf604fd004fe000df4000000000001ef711ff200ef400cf500bf70000004fd004fe004fe0000000000bf600003fe003ff000bf604fd000004fd00000000ef30000bfcbf7000003ff000006f55f64f700bf604fd004fe000df4000000000004ff003ff200ef600ef3008fa0000002fe006fe002ff2000000000bf600001ff105ff000bf604fd000004fd00000000ef30000bf62ff300002ff000006f55f64f700bf604fd001ff101ff1000000000002ff51bff200efc27fc0002ef5138000cf71cfe0009fb214a10000bf600000af81cff000bf604fd000004fd00000000ef30000bf607fc00000ef500006f55f64f700bf604fd0008f918f900000000000005cfd8ff200ef9dfc200002aeeb30002bfd9fe00007dfeb400000bf6000001afeaff000bf604fd000effffff800000ef30000bf600df700004cfff006f55f64f700bf604fd00007dfd70000000000000000000000000000000000000000000000000000000000000000000000000000003fe00000000000000000000000000ef200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005621af900000000000000000000000005fe0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000018dfd9100000000000000000000000effc400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ffffffffff000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ffffffffff000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ffffffffff000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ffffffffff000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000aefb000005f500000afea000000000000000ffffffffff0000000000000000000000000000000000000000001ff100000000000000000000000000000000000000000000000000000000000000000005fc10000005f50000001cf500000000000000ffffffffff0000000000000000000000000000000000000000001ff100000000000000000000000000000000000000000000000000000000000000000007f800000005f500000008f700000000000000ffffffffff0ef9dfc20002bfd9fe0000dfacef70008dec60003ffffffd000cf506fc004fe000df40ed00000de01ef706fe206fd000cf600afffffe000007f800000005f500000008f700000000000000ffffffffff0efc27fc000cf71cfe0000dfe4145008f8028400001ff100000cf506fc000ef302fe00bf00000fb004fe2ef5001ef302ff1000001dfe000007f800000005f500000008f700000000000000ffffffffff0ef600ef302fe006fe0000df7000000afa100000001ff100000cf506fc0009f707f9008f37f72f80008fef900009f908fb000000bff5000008f700000005f500000007f8000019ee931650ffffffffff0ef400cf504fd004fe0000df40000006fffc7000001ff100000cf506fc0004fb0bf4005f5afb5f60000cfd000003fe0df5000008ff8000002df400000005f500000004fd20005fffffff60ffffffffff0ef400cf504fd004fe0000df400000004aeff800001ff100000cf506fc0000ef1ee0003f8dae7f30002efe200000cf8fe000006ffa00000bff9000000005f5000000008ffb0056127dea10ffffffffff0ef600ef302fe006fe0000df400000000007fc00001ff100000cf506fc00009f9fa0000fcf3fcf0000bfbfc000006fff900004ffc10000002bf500000005f500000004fb20000000000000ffffffffff0efc27fc000cf71cfe0000df400000086207f900000ef400000af91cfc00005fff50000cfd0dfc0008fc0cf800001eff40000cfe2000000007f800000005f500000007f700000000000000ffffffffff0ef9dfb20002bfd9fe0000df400000018dfd91000005dffd0002cedafc00000efe100009fa09fa004ff303ff400009fd00000cfffffe000007f800000005f500000008f700000000000000ffffffffff0ef4000000000003fe000000000000000000000000000000000000000000000000000000000000000000000000000af8000000000000000007f800000005f500000008f700000000000000ffffffffff0ef4000000000003fe000000000000000000000000000000000000000000000000000000000000000000000000003ff2000000000000000005fb10000005f50000001bf500000000000000ffffffffff0ef4000000000003fe0000000000000000000000000000000000000000000000000000000000000000000000001ffd50000000000000000000aefb000005f500000afea000000000000000ffffffffff0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005f5000000000000000000000000ffffffffff000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ffffffffff";
const FONT_W: i32 = 160;
const FONT_H: i32 = 120;
const CELL_W: i32 = 10;
const CELL_H: i32 = 20;
const UI_SECONDS: f32 = 4.0;

/// (text, colour, when)
static UI_MSGS: Mutex<Vec<(String, [f32; 3], std::time::Instant)>> = Mutex::new(Vec::new());
/// GL objects for the text: (program, vao, vbo, texture, uScreen, uTex, uSolid)
static UI_GL: Mutex<Option<(u32, u32, u32, u32, i32, i32, i32)>> = Mutex::new(None);

/// Show a message on screen (and in the console).
pub(crate) fn ui_say(text: &str, colour: [f32; 3]) {
    info_f!("{}", text);
    if let Ok(mut m) = UI_MSGS.lock() {
        // a newer value of the same setting replaces the old line
        let key: String = text.split('=').next().unwrap_or("").to_string();
        if text.contains('=') {
            m.retain(|(t, _, _)| !t.starts_with(&key));
        }
        m.push((text.to_string(), colour, std::time::Instant::now()));
        let n = m.len();
        if n > 6 {
            m.drain(0..n - 6);
        }
    }
}
const UI_WHITE: [f32; 3] = [0.92, 0.92, 0.88];
const UI_RED: [f32; 3] = [1.0, 0.45, 0.4];

const UI_VS: &str = "#version 150
in vec2 aPos;
in vec2 aUV;
in vec4 aCol;
uniform vec2 uScreen;
out vec2 vUV;
out vec4 vCol;
void main() {
    vUV = aUV;
    vCol = aCol;
    gl_Position = vec4(aPos.x / uScreen.x * 2.0 - 1.0, 1.0 - aPos.y / uScreen.y * 2.0, 0.0, 1.0);
}
";
const UI_FS: &str = "#version 150
in vec2 vUV;
in vec4 vCol;
uniform sampler2D uTex;
out vec4 FragColor;
void main() {
    float a = texture(uTex, vUV).r;
    FragColor = vec4(vCol.rgb, vCol.a * a);
}
";

unsafe fn ui_setup(gl: &Gl) -> Option<(u32, u32, u32, u32, i32, i32, i32)> {
    unsafe {
        let compile = |kind: u32, src: &str| -> u32 {
            unsafe {
                let sh = (gl.create_shader)(kind);
                let c = std::ffi::CString::new(src).unwrap();
                let p = c.as_ptr();
                (gl.shader_source)(sh, 1, &p, std::ptr::null());
                (gl.compile_shader)(sh);
                sh
            }
        };
        let vs = compile(0x8B31, UI_VS);
        let fs = compile(0x8B30, UI_FS);
        let prog = (gl.create_program)();
        (gl.attach_shader)(prog, vs);
        (gl.attach_shader)(prog, fs);
        (gl.bind_attrib_location)(prog, 0, b"aPos\0".as_ptr() as *const c_char);
        (gl.bind_attrib_location)(prog, 1, b"aUV\0".as_ptr() as *const c_char);
        (gl.bind_attrib_location)(prog, 2, b"aCol\0".as_ptr() as *const c_char);
        (gl.link_program)(prog);
        let mut ok = 0;
        (gl.get_programiv)(prog, 0x8B82, &mut ok);
        if ok == 0 {
            warn_f!("Cruor: on-screen text shader didn't build - messages stay in the console");
            return None;
        }
        let u = |n: &[u8]| unsafe { (gl.get_uniform_location)(prog, n.as_ptr() as *const c_char) };
        let (us, ut, uso) = (u(b"uScreen\0"), u(b"uTex\0"), u(b"uSolid\0"));
        // font texture (8-bit, expanded from the 4-bit data)
        let mut px = Vec::with_capacity((FONT_W * FONT_H) as usize);
        let hb = FONT_HEX.as_bytes();
        let hexv = |c: u8| -> u8 { if c.is_ascii_digit() { c - b'0' } else { c - b'a' + 10 } };
        for i in (0..hb.len()).step_by(2) {
            let byte = (hexv(hb[i]) << 4) | hexv(hb[i + 1]);
            px.push((byte >> 4) * 17);
            px.push((byte & 15) * 17);
        }
        let mut tex = 0;
        (gl.gen_textures)(1, &mut tex);
        let mut prev_tex = 0;
        (gl.get_integerv)(GL_TEXTURE_BINDING_2D, &mut prev_tex);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        let mut prev_align = 4;
        (gl.get_integerv)(0x0CF5, &mut prev_align); // UNPACK_ALIGNMENT
        (gl.pixel_storei)(0x0CF5, 1);
        (gl.tex_image_2d)(GL_TEXTURE_2D, 0, 0x8229 /* R8 */, FONT_W, FONT_H, 0, 0x1903 /* RED */, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
        (gl.pixel_storei)(0x0CF5, prev_align);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        (gl.bind_texture)(GL_TEXTURE_2D, prev_tex as u32);
        let (mut vao, mut vbo) = (0, 0);
        (gl.gen_vertex_arrays)(1, &mut vao);
        (gl.gen_buffers)(1, &mut vbo);
        Some((prog, vao, vbo, tex, us, ut, uso))
    }
}

/// Draw the current messages into the window (called at the end of each frame).
unsafe fn ui_draw(gl: &Gl, win_w: i32, win_h: i32) {
    let now = std::time::Instant::now();
    let lines: Vec<(String, [f32; 3], f32)> = match UI_MSGS.lock() {
        Ok(mut m) => {
            m.retain(|(_, _, t)| (now - *t).as_secs_f32() < UI_SECONDS);
            m.iter()
                .map(|(t, c, at)| {
                    let age = (now - *at).as_secs_f32();
                    (t.clone(), *c, (1.0 - ((age - (UI_SECONDS - 1.0)).max(0.0))).clamp(0.0, 1.0))
                })
                .collect()
        }
        Err(_) => return,
    };
    if lines.is_empty() || win_w <= 0 || win_h <= 0 {
        return;
    }
    let res = {
        let mut g = match UI_GL.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if g.is_none() {
            *g = unsafe { ui_setup(gl) };
            if g.is_none() {
                *g = Some((0, 0, 0, 0, -1, -1, -1));
            }
        }
        (*g).unwrap()
    };
    if res.0 == 0 {
        return;
    }
    // bigger text on big screens
    let scale = if win_h >= 1400 { 1.5f32 } else { 1.0 };
    let (cw, ch) = (CELL_W as f32 * scale, CELL_H as f32 * scale);
    let (x0, y0) = (14.0f32, 14.0f32);
    let mut v: Vec<f32> = Vec::new();
    let mut quad = |x: f32, y: f32, w: f32, h: f32, u0: f32, v0: f32, u1: f32, v1: f32, c: [f32; 4]| {
        let pts = [(x, y, u0, v0), (x + w, y, u1, v0), (x + w, y + h, u1, v1), (x, y, u0, v0), (x + w, y + h, u1, v1), (x, y + h, u0, v1)];
        for (px, py, pu, pv) in pts {
            v.extend_from_slice(&[px, py, pu, pv, c[0], c[1], c[2], c[3]]);
        }
    };
    // solid cell (last in the atlas) for the panel background
    let solid = |_: ()| {
        let i = 95;
        let (cx, cy) = ((i % 16) * CELL_W, (i / 16) * CELL_H);
        (
            (cx as f32 + CELL_W as f32 * 0.5) / FONT_W as f32,
            (cy as f32 + CELL_H as f32 * 0.5) / FONT_H as f32,
        )
    };
    let (su, sv) = solid(());
    for (k, (text, col, a)) in lines.iter().enumerate() {
        let y = y0 + k as f32 * (ch + 2.0 * scale);
        let n = text.chars().count().min(120) as f32;
        quad(x0 - 6.0, y - 1.0, n * cw + 12.0, ch + 2.0, su, sv, su, sv, [0.0, 0.0, 0.0, 0.55 * a]);
        for (j, chr) in text.chars().take(120).enumerate() {
            let code = chr as i32;
            if !(33..=126).contains(&code) {
                continue;
            }
            let i = code - 32;
            let (cx, cy) = ((i % 16) * CELL_W, (i / 16) * CELL_H);
            quad(
                x0 + j as f32 * cw, y, cw, ch,
                cx as f32 / FONT_W as f32, cy as f32 / FONT_H as f32,
                (cx + CELL_W) as f32 / FONT_W as f32, (cy + CELL_H) as f32 / FONT_H as f32,
                [col[0], col[1], col[2], *a],
            );
        }
    }
    unsafe {
        // save what we touch
        let (mut prog, mut vao, mut buf, mut fbo, mut active, mut tex) = (0, 0, 0, 0, 0, 0);
        let mut vp = [0i32; 4];
        let mut bs = [0i32; 4];
        let mut be = [0i32; 2];
        let mut cm = [1u8; 4];
        (gl.get_integerv)(GL_CURRENT_PROGRAM, &mut prog);
        (gl.get_integerv)(GL_VERTEX_ARRAY_BINDING, &mut vao);
        (gl.get_integerv)(GL_ARRAY_BUFFER_BINDING, &mut buf);
        (gl.get_integerv)(GL_DRAW_FRAMEBUFFER_BINDING, &mut fbo);
        (gl.get_integerv)(GL_VIEWPORT, vp.as_mut_ptr());
        (gl.get_integerv)(GL_ACTIVE_TEXTURE, &mut active);
        (gl.active_texture)(GL_TEXTURE0 + BG_UNIT);
        (gl.get_integerv)(GL_TEXTURE_BINDING_2D, &mut tex);
        (gl.get_integerv)(GL_BLEND_SRC_RGB, &mut bs[0]);
        (gl.get_integerv)(GL_BLEND_DST_RGB, &mut bs[1]);
        (gl.get_integerv)(GL_BLEND_SRC_ALPHA, &mut bs[2]);
        (gl.get_integerv)(GL_BLEND_DST_ALPHA, &mut bs[3]);
        (gl.get_integerv)(0x8009, &mut be[0]);
        (gl.get_integerv)(0x883D, &mut be[1]);
        (gl.get_booleanv)(GL_COLOR_WRITEMASK, cm.as_mut_ptr());
        let caps = [GL_BLEND, GL_DEPTH_TEST, GL_CULL_FACE, GL_SCISSOR_TEST, GL_STENCIL_TEST, 0x8DB9 /* FRAMEBUFFER_SRGB */];
        let was: Vec<u8> = caps.iter().map(|&c| unsafe { (gl.is_enabled)(c) }).collect();
        gl_clear_errors(gl);

        (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
        (gl.viewport)(0, 0, win_w, win_h);
        for &c in caps.iter() {
            (gl.disable)(c);
        }
        (gl.enable)(GL_BLEND);
        (gl.blend_equation_separate)(0x8006, 0x8006);
        (gl.blend_func_separate)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ZERO, GL_ONE);
        (gl.color_mask)(1, 1, 1, 1);
        (gl.use_program)(res.0);
        (gl.uniform2f)(res.4, win_w as f32, win_h as f32);
        (gl.uniform1i)(res.5, BG_UNIT as i32);
        (gl.bind_texture)(GL_TEXTURE_2D, res.3);
        (gl.bind_vertex_array)(res.1);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, res.2);
        (gl.buffer_data)(GL_ARRAY_BUFFER, (v.len() * 4) as isize, v.as_ptr() as *const c_void, GL_STREAM_DRAW);
        let stride = 8 * 4;
        (gl.vertex_attrib_pointer)(0, 2, GL_FLOAT, 0, stride, std::ptr::null());
        (gl.enable_vertex_attrib_array)(0);
        (gl.vertex_attrib_pointer)(1, 2, GL_FLOAT, 0, stride, 8 as *const c_void);
        (gl.enable_vertex_attrib_array)(1);
        (gl.vertex_attrib_pointer)(2, 4, GL_FLOAT, 0, stride, 16 as *const c_void);
        (gl.enable_vertex_attrib_array)(2);
        (gl.draw_arrays)(GL_TRIANGLES, 0, (v.len() / 8) as i32);

        // restore
        (gl.bind_vertex_array)(vao as u32);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, buf as u32);
        (gl.use_program)(prog as u32);
        (gl.bind_texture)(GL_TEXTURE_2D, tex as u32);
        (gl.active_texture)(active as u32);
        (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, fbo as u32);
        (gl.viewport)(vp[0], vp[1], vp[2], vp[3]);
        (gl.blend_equation_separate)(be[0] as u32, be[1] as u32);
        (gl.blend_func_separate)(bs[0] as u32, bs[1] as u32, bs[2] as u32, bs[3] as u32);
        (gl.color_mask)(cm[0], cm[1], cm[2], cm[3]);
        for (k, &c) in caps.iter().enumerate() {
            if was[k] != 0 {
                (gl.enable)(c)
            } else {
                (gl.disable)(c)
            }
        }
        gl_clear_errors(gl);
    }
}

/// Latest flying-drop geometry (built once per frame by prepare_frame).
static LAST_VERTS: Mutex<Vec<f32>> = Mutex::new(Vec::new());
/// Frame number of the last successful in-scene draw (0 = never).
static IN_SCENE_FRAME: AtomicU32 = AtomicU32::new(0);
static IN_SCENE_FAILED: AtomicBool = AtomicBool::new(false);

/// Same camera? (all matrix entries equal within a small relative tolerance)
fn same_camera(a: &Mat4, b: &Mat4) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= 1e-3 * (1.0 + x.abs().max(y.abs())))
}

/// Once in a while, say so when the game drew a copy of the scene with more than one
/// camera (the blood uses the one most draws used).
fn report_cameras(votes: &[([i32; 4], Vec<(Mat4, [f32; 32], u32)>)]) {
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);

    if RELEASE {
        return;
    }
    for (v, list) in votes.iter() {
        // only the real scene views (not the game's tiny off-screen jobs)
        if list.len() < 2 || v[2] < 64 || v[3] < 64 {
            continue;
        }
        if let Ok(mut t) = LAST.lock() {
            if t.map(|t| t.elapsed().as_secs_f32() < 10.0).unwrap_or(false) {
                return;
            }
            *t = Some(std::time::Instant::now());
        }
        let total: u32 = list.iter().map(|e| e.2).sum();
        let mut counts: Vec<u32> = list.iter().map(|e| e.2).collect();
        counts.sort_unstable_by(|a, b| b.cmp(a));
        info_f!(
            "Cruor: the game drew the scene area {}x{} with {} different cameras this frame (draws per camera: {:?} of {}); blood uses the main one",
            v[2], v[3], list.len(), counts, total
        );
        return;
    }
}

/// The camera (view-projection) a copy of the scene was drawn with.
fn tile_view_proj(m: &[f32; 32]) -> Option<Mat4> {
    let mut obj_to_world: Mat4 = [0.0; 16];
    let mut obj_to_clip: Mat4 = [0.0; 16];
    obj_to_world.copy_from_slice(&m[0..16]);
    obj_to_clip.copy_from_slice(&m[16..32]);
    Some(mat_mul(&obj_to_clip, &mat_inverse(&obj_to_world)?))
}

/// What the game's scene buffer is (worked out once per buffer).
#[derive(Clone)]
struct SceneInfo {
    fbo: u32,
    samples: i32,
    color_fmt: i32, // matching internal format for a copy of its colour
    has_depth: bool,
    desc: String,
}
static SCENE_INFO: Mutex<Option<SceneInfo>> = Mutex::new(None);
static SCENE_LOGGED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// The internal format that matches a colour attachment's description.
fn infer_color_format(ctype: i32, r: i32, g: i32, b: i32, a: i32, srgb: bool) -> Option<i32> {
    let _ = g;
    match ctype {
        0x1406 => match (r, b, a) {
            // FLOAT
            (16, _, 0) => Some(0x881B),  // RGB16F
            (16, _, _) => Some(0x881A),  // RGBA16F
            (32, _, 0) => Some(0x8815),  // RGB32F
            (32, _, _) => Some(0x8814),  // RGBA32F
            (11, 10, _) => Some(0x8C3A), // R11F_G11F_B10F
            _ => None,
        },
        0x8C17 => match (r, a) {
            // UNSIGNED_NORMALIZED
            (8, 0) => Some(if srgb { 0x8C41 } else { 0x8051 }), // SRGB8 / RGB8
            (8, _) => Some(if srgb { 0x8C43 } else { 0x8058 }), // SRGB8_ALPHA8 / RGBA8
            (10, 2) => Some(0x8059),                            // RGB10_A2
            (16, 0) => Some(0x8054),                            // RGB16
            (16, _) => Some(0x805B),                            // RGBA16
            _ => None,
        },
        _ => None,
    }
}

/// Look at the currently bound draw framebuffer (the scene): samples, colour format, depth.
unsafe fn describe_scene(gl: &Gl, fbo: u32) -> SceneInfo {
    unsafe {
        let mut samples = 0;
        (gl.get_integerv)(0x80A9, &mut samples); // SAMPLES (of the bound draw framebuffer)
        // colour attachment: COLOR_ATTACHMENT0 for a framebuffer object, BACK_LEFT for the window
        let (col, dep, dst) = if fbo == 0 { (0x0402u32, 0x1801u32, 0x1802u32) } else { (0x8CE0u32, 0x8D00u32, 0x821Au32) };
        let q = |att: u32, pname: u32| -> i32 {
            let mut v = 0;
            unsafe { (gl.get_framebuffer_attachment_parameteriv)(GL_DRAW_FRAMEBUFFER, att, pname, &mut v) };
            v
        };
        let ctype = q(col, 0x8211); // COMPONENT_TYPE
        let (r, g, b, a) = (q(col, 0x8212), q(col, 0x8213), q(col, 0x8214), q(col, 0x8215));
        let srgb = q(col, 0x8210) == 0x8C40; // COLOR_ENCODING == SRGB
        // (plain depth first; only ask about depth+stencil if there's no plain depth)
        let mut depth_bits = q(dep, 0x8216); // DEPTH_SIZE
        let mut depth_obj = if fbo == 0 { depth_bits } else { q(dep, 0x8CD0) }; // OBJECT_TYPE
        if depth_obj == 0 && fbo != 0 {
            depth_obj = q(dst, 0x8CD0);
            depth_bits = q(dst, 0x8216);
        }
        for _ in 0..8 {
            if (gl.get_error)() == 0 {
                break;
            }
        }
        let fmt = infer_color_format(ctype, r, g, b, a, srgb).unwrap_or(0);
        let desc = format!(
            "{} - samples {}, colour {} {}/{}/{}/{}{}, depth {}",
            if fbo == 0 { "the window".to_string() } else { format!("buffer {}", fbo) },
            samples,
            match ctype { 0x1406 => "float", 0x8C17 => "unorm", 0 => "none", _ => "other" },
            r, g, b, a,
            if srgb { " sRGB" } else { "" },
            if depth_obj != 0 { format!("yes ({} bit)", depth_bits) } else { "NO".to_string() }
        );
        SceneInfo { fbo, samples, color_fmt: fmt, has_depth: depth_obj != 0, desc }
    }
}

// ============================================================
// DIAGNOSTICS (test build)
//   F11: marker test - a solid magenta square in each scene copy, at the exact moment
//        the blood is drawn (no depth test, no blending).
//   F12: one-frame trace of how the game draws a frame.
// ============================================================
/// Release build: the test keys (F9 probe, F11 marker, F12 trace) and the red on-screen
/// test messages are off. Set to false for a test build.
const RELEASE: bool = true;
static MARKER_ON: AtomicBool = AtomicBool::new(false);
static TRACE_ARMED: AtomicBool = AtomicBool::new(false);
static TRACE_ACTIVE: AtomicBool = AtomicBool::new(false);
static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static TRACE_DRAWS: AtomicU32 = AtomicU32::new(0);
static REAL_CLEAR: AtomicUsize = AtomicUsize::new(0);
static REAL_BLIT: AtomicUsize = AtomicUsize::new(0);
static REAL_BIND_TEXTURE: AtomicUsize = AtomicUsize::new(0);
static REAL_DRAW_ARRAYS: AtomicUsize = AtomicUsize::new(0);
/// Texture names that are attached to framebuffers (so we can see who reads a buffer's image).
static FB_TEXTURES: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new()); // (texture, fbo)
/// The framebuffer currently bound for drawing.
static CUR_DRAW_FBO: AtomicU32 = AtomicU32::new(0);
/// Which texture the game has on each texture unit (tracked from its own calls).
static CUR_UNIT: AtomicU32 = AtomicU32::new(0);
#[allow(clippy::declare_interior_mutable_const)]
const ZERO_U32: AtomicU32 = AtomicU32::new(0);
static UNIT_TEX: [AtomicU32; 32] = [ZERO_U32; 32];
static REAL_ACTIVE_TEXTURE: AtomicUsize = AtomicUsize::new(0);
static REAL_FB_RENDERBUFFER: AtomicUsize = AtomicUsize::new(0);

/// The game attaches/detaches a renderbuffer image (its scene depth is one). The first
/// such change after the scene is drawn = the scene is finished: draw the blood first.
unsafe extern "system" fn my_fb_renderbuffer(target: u32, attach: u32, rbtarget: u32, rb: u32) {
    if target == GL_FRAMEBUFFER || target == GL_DRAW_FRAMEBUFFER {
        before_snapshot("the game changes the scene's images");
    }
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace_flush_draws();
        trace(format!(
            "   >> ATTACH renderbuffer {} to buffer {} as 0x{:X}",
            rb, CUR_DRAW_FBO.load(Ordering::Relaxed), attach
        ));
    }
    let real: unsafe extern "system" fn(u32, u32, u32, u32) = unsafe { std::mem::transmute(REAL_FB_RENDERBUFFER.load(Ordering::Relaxed)) };
    unsafe { real(target, attach, rbtarget, rb) }
}
static REAL_FB_TEXTURE_2D: AtomicUsize = AtomicUsize::new(0);
/// Textures used by the window's draws this frame / last frame (unit contents at each draw).
static WINDOW_UNIT_TEX: Mutex<(Vec<u32>, Vec<u32>)> = Mutex::new((Vec::new(), Vec::new()));

unsafe extern "system" fn my_active_texture(unit: u32) {
    CUR_UNIT.store(unit.wrapping_sub(0x84C0), Ordering::Relaxed);
    let real: unsafe extern "system" fn(u32) = unsafe { std::mem::transmute(REAL_ACTIVE_TEXTURE.load(Ordering::Relaxed)) };
    unsafe { real(unit) }
}
unsafe extern "system" fn my_fb_texture_2d(target: u32, attach: u32, textarget: u32, tex: u32, level: i32) {
    // The game swaps the scene buffer's image to combine its supersampled copies into
    // the picture it shows (texture 27 here). That's the moment the scene is finished:
    // draw the blood into it now, before it's combined. (Only fires for the scene buffer
    // in the middle of a full scene visit - see before_snapshot.)
    if target == GL_FRAMEBUFFER || target == GL_DRAW_FRAMEBUFFER {
        // (any attachment: the game detaches the depth before swapping the colour)
        before_snapshot("the game combines its scene copies");
        if attach == 0x8CE0 && tex != 0 {
            if let Ok(mut sn) = SNAP.lock() {
                if let Some(s) = sn.as_mut() {
                    if CUR_DRAW_FBO.load(Ordering::Relaxed) == s.fbo && tex != s.scene_tex {
                        s.combined = tex;
                    }
                }
            }
        }
    }
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace_flush_draws();
        trace(format!(
            "   >> ATTACH texture {} (level {}) to buffer {} as {}",
            tex, level, CUR_DRAW_FBO.load(Ordering::Relaxed),
            match attach { 0x8D00 => "depth".to_string(), 0x821A => "depth+stencil".to_string(), a if (0x8CE0..0x8CF0).contains(&a) => format!("layer {}", a - 0x8CE0), a => format!("0x{:X}", a) }
        ));
    }
    if let Ok(mut v) = FB_TEXTURES.lock() {
        let fbo = CUR_DRAW_FBO.load(Ordering::Relaxed);
        v.retain(|(t, _)| *t != tex);
        v.push((tex, fbo));
    }
    let real: unsafe extern "system" fn(u32, u32, u32, u32, i32) = unsafe { std::mem::transmute(REAL_FB_TEXTURE_2D.load(Ordering::Relaxed)) };
    unsafe { real(target, attach, textarget, tex, level) }
}

/// A draw in a small post-processing step (buffer 4 or the window): log which textures it uses.
fn note_post_draw() {
    let fbo = CUR_DRAW_FBO.load(Ordering::Relaxed);
    if fbo != 0 && fbo != 4 {
        return;
    }
    let units: Vec<u32> = (0..4).map(|u| UNIT_TEX[u].load(Ordering::Relaxed)).collect();
    if fbo == 0 {
        if let Ok(mut w) = WINDOW_UNIT_TEX.lock() {
            for t in units.iter() {
                if *t != 0 && !w.0.contains(t) && w.0.len() < 16 {
                    w.0.push(*t);
                }
            }
        }
    }
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        let vp = with_fs(|fs| fs.cur_viewport).unwrap_or([0; 4]);
        trace_flush_draws();
        trace(format!(
            "   draw in buffer {} (area {}x{}, program {}) using textures on units 0-3: {:?}",
            fbo, vp[2], vp[3], CURRENT_PROGRAM.load(Ordering::Relaxed), units
        ));
    }
}
/// Textures the game read while drawing into the window (the final picture), this and last frame.
static WINDOW_READS: Mutex<(Vec<u32>, Vec<u32>)> = Mutex::new((Vec::new(), Vec::new()));

fn trace(line: String) {
    if !TRACE_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut t) = TRACE.lock() {
        if t.len() < 2000 {
            t.push(line);
        }
    }
}

/// One visit to a buffer, summarised in one line when it ends.
struct Visit {
    fbo: u32,
    areas: Vec<[i32; 4]>,
    draws: u32,
    clears: Vec<&'static str>,
    reads: Vec<u32>,
}
static VISIT: Mutex<Option<Visit>> = Mutex::new(None);

fn visit_start(fbo: u32) {
    if let Ok(mut v) = VISIT.lock() {
        *v = Some(Visit { fbo, areas: Vec::new(), draws: 0, clears: Vec::new(), reads: Vec::new() });
    }
}
fn visit_with(f: impl FnOnce(&mut Visit)) {
    if !TRACE_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut v) = VISIT.lock() {
        if let Some(v) = v.as_mut() {
            f(v);
        }
    }
}
/// End the current visit: write its summary line.
fn trace_flush_draws() {
    let v = match VISIT.lock() {
        Ok(mut v) => v.take(),
        Err(_) => None,
    };
    if let Some(v) = v {
        if v.draws == 0 && v.clears.is_empty() && v.reads.is_empty() && v.areas.is_empty() {
            visit_start(v.fbo);
            return; // nothing happened in it
        }
        let areas = v.areas.iter().map(|a| format!("{},{} {}x{}", a[0], a[1], a[2], a[3])).collect::<Vec<_>>().join(" | ");
        trace(format!(
            "buffer {}{}: {} draws; areas [{}]; clears [{}]; reads images of buffers {:?}",
            v.fbo,
            if v.fbo == 0 { " (the window)" } else { "" },
            v.draws,
            areas,
            v.clears.join(", "),
            v.reads
        ));
        // keep going in the same buffer (e.g. after a copy) until the next switch
        visit_start(v.fbo);
    }
}

unsafe extern "system" fn my_clear(mask: u32) {
    visit_with(|v| {
        let c = match (mask & 0x4000 != 0, mask & 0x100 != 0) {
            (true, true) => "colour+depth",
            (true, false) => "colour",
            (false, true) => "depth",
            _ => "other",
        };
        v.clears.push(c);
    });
    let real: unsafe extern "system" fn(u32) = unsafe { std::mem::transmute(REAL_CLEAR.load(Ordering::Relaxed)) };
    unsafe { real(mask) }
}

unsafe extern "system" fn my_blit(sx0: i32, sy0: i32, sx1: i32, sy1: i32, dx0: i32, dy0: i32, dx1: i32, dy1: i32, mask: u32, filter: u32) {
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace_flush_draws();
        let (mut rf, mut df) = (0, 0);
        unsafe {
            let gi: unsafe extern "system" fn(u32, *mut i32) = std::mem::transmute(gl_resolve("glGetIntegerv"));
            gi(GL_READ_FRAMEBUFFER_BINDING, &mut rf);
            gi(GL_DRAW_FRAMEBUFFER_BINDING, &mut df);
        }
        trace(format!(
            "   >> COPY buffer {} [{},{} - {},{}] -> buffer {} [{},{} - {},{}] ({}{}{}, {})",
            rf, sx0, sy0, sx1, sy1, df, dx0, dy0, dx1, dy1,
            if mask & 0x4000 != 0 { "colour " } else { "" },
            if mask & 0x100 != 0 { "depth " } else { "" },
            if mask & 0x400 != 0 { "stencil " } else { "" },
            if filter == GL_NEAREST { "nearest" } else { "linear" }
        ));
    }
    let real: unsafe extern "system" fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32) =
        unsafe { std::mem::transmute(REAL_BLIT.load(Ordering::Relaxed)) };
    unsafe { real(sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask, filter) }
}

unsafe extern "system" fn my_bind_texture(target: u32, tex: u32) {
    if target == GL_TEXTURE_2D {
        let u = CUR_UNIT.load(Ordering::Relaxed) as usize;
        if u < 32 {
            UNIT_TEX[u].store(tex, Ordering::Relaxed);
        }
    }
    if tex != 0 && CUR_DRAW_FBO.load(Ordering::Relaxed) == 0 && target == GL_TEXTURE_2D {
        if let Ok(mut w) = WINDOW_READS.lock() {
            if !w.0.contains(&tex) && w.0.len() < 16 {
                w.0.push(tex);
            }
        }
    }
    if TRACE_ACTIVE.load(Ordering::Relaxed) && tex != 0 {
        let owner = FB_TEXTURES.lock().ok().and_then(|v| v.iter().find(|(t, _)| *t == tex).map(|(_, f)| *f));
        if let Some(fbo) = owner {
            visit_with(|v| {
                if !v.reads.contains(&fbo) {
                    v.reads.push(fbo);
                }
            });
        }
    }
    let real: unsafe extern "system" fn(u32, u32) = unsafe { std::mem::transmute(REAL_BIND_TEXTURE.load(Ordering::Relaxed)) };
    unsafe { real(target, tex) }
}

/// A finished supersampled scene waiting for the game to combine its copies.
#[derive(Clone)]
struct Snap {
    fbo: u32,         // the scene buffer
    tile: [i32; 4],   // copy 0's area in it
    cam: [f32; 32],   // copy 0's camera
    scene_tex: u32,   // the scene buffer's own colour image
    combined: u32,    // the full-size picture the game combines the copies into
    depth_ok: bool,   // copy 0's depth saved in our depth store
}
static SNAP: Mutex<Option<Snap>> = Mutex::new(None);
/// Our framebuffer for drawing into the combined picture, with a saved copy of the
/// scene's depth: (framebuffer, depth renderbuffer, size, depth format).
static CMB: Mutex<(u32, u32, (i32, i32), i32)> = Mutex::new((0, 0, (0, 0), 0));
static CMB_LOGGED: AtomicBool = AtomicBool::new(false);

/// The scene is finished (first image change). Save copy 0's depth before the game
/// detaches it; the blood goes into the combined picture once the game has made it.
unsafe fn capture_scene_depth(scene: u32, tiles: &[([i32; 4], [f32; 32])]) {
    let full = tiles.iter().map(|(v, _)| v[2] * v[3]).max().unwrap_or(0);
    let (tile, cam) = match tiles.iter().filter(|(v, _)| v[2] * v[3] == full).min_by_key(|(v, _)| (v[1], v[0])) {
        Some((v, c)) => (*v, *c),
        None => return,
    };
    let cell = match RENDERER.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    let r = match cell.0.as_ref() {
        Some(r) => r,
        None => return,
    };
    let gl = &r.gl;
    unsafe {
        // (the game's camera block is read back from the GPU only for the F7 check: reading a
        // buffer the card is still using makes the CPU wait for the card - a full stall)
        if DROP_CHECK.load(Ordering::Relaxed) {
            read_game_global(gl);
        }
        gl_clear_errors_v(gl);
        // the scene buffer's colour image and depth format don't change from frame to frame:
        // asked once per buffer and size (and again on verification frames)
        let cached = DEPTH_INFO.lock().ok().and_then(|c| *c).filter(|c| c.0 == scene && c.1 == tile[2] && c.2 == tile[3] && !gl_verify());
        let (scene_tex, fmt, is_ds) = if let Some(c) = cached { (c.3, c.4, c.5) } else {
        let q = |att: u32, pname: u32| -> i32 {
            let mut v = 0;
            unsafe { (gl.get_framebuffer_attachment_parameteriv)(GL_DRAW_FRAMEBUFFER, att, pname, &mut v) };
            v
        };
        // the scene buffer's colour image (so we can tell the combined picture apart)
        let scene_tex = if q(0x8CE0, 0x8CD0) == 0x1702 { q(0x8CE0, 0x8CD1) as u32 } else { 0 };
        // its depth format
        let (mut fmt, mut is_ds) = (0, false);
        // (plain depth first: asking a depth-only buffer about DEPTH_STENCIL is an error)
        for att in [0x8D00u32, 0x821A] {
            let ty = q(att, 0x8CD0);
            gl_clear_errors(gl);
            if ty == 0 {
                continue;
            }
            let name = q(att, 0x8CD1) as u32;
            if ty == 0x8D41 {
                let mut prev = 0;
                (gl.get_integerv)(0x8CA7, &mut prev);
                (gl.bind_renderbuffer)(0x8D41, name);
                (gl.get_renderbuffer_parameteriv)(0x8D41, 0x8D44, &mut fmt);
                (gl.bind_renderbuffer)(0x8D41, prev as u32);
            } else if ty == 0x1702 {
                let mut prev = 0;
                (gl.get_integerv)(GL_TEXTURE_BINDING_2D, &mut prev);
                (gl.bind_texture)(GL_TEXTURE_2D, name);
                (gl.get_tex_level_parameteriv)(GL_TEXTURE_2D, 0, 0x1003, &mut fmt);
                (gl.bind_texture)(GL_TEXTURE_2D, prev as u32);
            }
            is_ds = att == 0x821A || fmt == 0x88F0 || fmt == 0x8CAD;
            break;
        }
        if let Ok(mut c) = DEPTH_INFO.lock() {
            *c = Some((scene, tile[2], tile[3], scene_tex, fmt, is_ds));
        }
        (scene_tex, fmt, is_ds)
        };
        let mut depth_ok = false;
        if fmt != 0 {
            let (w, h) = (tile[2], tile[3]);
            // (the game's bindings, as its hooked glBindFramebuffer calls left them)
            let (prev_read, prev_draw) = (CUR_READ_FBO.load(Ordering::Relaxed) as i32, CUR_DRAW_FBO.load(Ordering::Relaxed) as i32);
            let mut c = CMB.lock().unwrap();
            if c.0 == 0 {
                (gl.gen_framebuffers)(1, &mut c.0);
                (gl.gen_renderbuffers)(1, &mut c.1);
            }
            if c.2 != (w, h) || c.3 != fmt {
                let mut prev_rb = 0;
                (gl.get_integerv)(0x8CA7, &mut prev_rb);
                (gl.bind_renderbuffer)(0x8D41, c.1);
                (gl.renderbuffer_storage)(0x8D41, fmt as u32, w, h);
                (gl.bind_renderbuffer)(0x8D41, prev_rb as u32);
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, c.0);
                (gl.framebuffer_renderbuffer)(GL_DRAW_FRAMEBUFFER, if is_ds { 0x821A } else { 0x8D00 }, 0x8D41, c.1);
                c.2 = (w, h);
                c.3 = fmt;
            }
            (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, c.0);
            (gl.framebuffer_texture_2d)(GL_DRAW_FRAMEBUFFER, 0x8CE0, GL_TEXTURE_2D, 0, 0);
            (gl.draw_buffer)(0);
            (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, scene);
            gl_clear_errors_v(gl); // only the copy's own result counts
            (gl.blit_framebuffer)(tile[0], tile[1], tile[0] + w, tile[1] + h, 0, 0, w, h, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
            let err = gl_err_v(gl);
            depth_ok = err == 0;
            if !depth_ok && test_log_ok() {
                error_f!("Cruor TEST: couldn't save the scene's depth (format 0x{:X}, error 0x{:X})", fmt, err);
            }
            (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, prev_read as u32);
            (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, prev_draw as u32);
            gl_clear_errors_v(gl);
        } else if test_log_ok() {
            error_f!("Cruor TEST: the scene has no depth at the moment it's finished");
        }
        if let Ok(mut sn) = SNAP.lock() {
            *sn = Some(Snap { fbo: scene, tile, cam, scene_tex, combined: 0, depth_ok });
        }
    }
}

/// The game's own camera block ("Global", std140, shared by ~500 scene shaders), read
/// from the buffer the game bound for its shaders when the scene was finished:
/// ViewPos @0, ViewSiz @16, ViewPrj @24, CamMat @32, SdwMat @96, VXGIScl @160,
/// Ambient @176, TimeDlt @192, ViewRcp @200, FogColr @208, FogPos @224, BufrPos @240,
/// Wetness @248. (raw floats, program it was read through, binding, buffer, offset)
static GAME_GLOBAL: Mutex<Option<([f32; 64], u32, u32, u32, i32)>> = Mutex::new(None);

/// Read the game's Global block as it is bound right now (call while the game's state
/// for the scene is still current).
unsafe fn read_game_global(gl: &Gl) {
    unsafe {
        // the game's program in use, or any scene program we know of
        let mut progs: Vec<u32> = vec![CURRENT_PROGRAM.load(Ordering::Relaxed)];
        if let Ok(g) = CAM.lock() {
            if let Some(c) = g.as_ref() {
                progs.extend(c.main_programs.iter().take(8).copied());
            }
        }
        gl_clear_errors(gl);
        for prog in progs {
            if prog == 0 {
                continue;
            }
            let idx = (gl.get_uniform_block_index)(prog, b"Global\0".as_ptr() as *const c_char);
            if idx == u32::MAX || (gl.get_error)() != 0 {
                continue;
            }
            let mut binding = -1;
            (gl.get_active_uniform_blockiv)(prog, idx, 0x8A3F /* UNIFORM_BLOCK_BINDING */, &mut binding);
            if binding < 0 || (gl.get_error)() != 0 {
                continue;
            }
            let (mut buf, mut start) = (0, 0);
            (gl.get_integeri_v)(0x8A28 /* UNIFORM_BUFFER_BINDING */, binding as u32, &mut buf);
            (gl.get_integeri_v)(0x8A29 /* UNIFORM_BUFFER_START */, binding as u32, &mut start);
            if buf == 0 || (gl.get_error)() != 0 {
                continue;
            }
            let mut prev = 0;
            (gl.get_integerv)(0x8F36 /* COPY_READ_BUFFER binding */, &mut prev);
            (gl.bind_buffer)(0x8F36, buf as u32);
            let mut data = [0.0f32; 64];
            (gl.get_buffer_sub_data)(0x8F36, start as isize, 256, data.as_mut_ptr() as *mut c_void);
            let err = (gl.get_error)();
            (gl.bind_buffer)(0x8F36, prev as u32);
            gl_clear_errors(gl);
            if err == 0 {
                if let Ok(mut g) = GAME_GLOBAL.lock() {
                    *g = Some((data, prog, binding as u32, buf as u32, start));
                }
                return;
            }
        }
    }
}

/// FLYING-BLOOD CHECK (F7): for a few drops, where they land on screen with the camera
/// used, their depth vs the game's saved scene depth at that pixel, and the colour
/// there after drawing. Tells apart: wrong camera / hidden by the depth test / removed.
static DROP_CHECK: AtomicBool = AtomicBool::new(false);
/// F7: count on the GPU how many pixels the next blood draw writes (and covers).
static DRAW_COUNT: AtomicBool = AtomicBool::new(false);
static QUERIES: Mutex<[u32; 2]> = Mutex::new([0, 0]);
unsafe fn drop_check(gl: &Gl, fbo: u32, w: i32, h: i32, cam: &[f32; 32]) {
    let vp = match tile_view_proj(cam) {
        Some(v) => v,
        None => {
            info_f!("Cruor CHECK: camera matrix can't be used (not invertible)");
            return;
        }
    };
    let eye = mat_inverse(&vp).map(|inv| {
        let e = mat_mul_vec(&inv, [0.0, 0.0, 1.0, 0.0]);
        if e[3].abs() > 1e-9 { [e[0] / e[3], e[1] / e[3], e[2] / e[3]] } else { [f32::NAN; 3] }
    });
    let centres: Vec<[f32; 3]> = match LAST_VERTS.lock() {
        Ok(v) => v.chunks(54).filter(|c| c.len() >= 4 && c[3] > 0.0).map(|c| [c[0], c[1], c[2]]).collect(),
        Err(_) => return,
    };
    info_f!(
        "Cruor CHECK ===== {} flying drops; picture {}x{}; camera at {:?} =====",
        centres.len(), w, h, eye.map(|e| [e[0].round(), e[1].round(), e[2].round()])
    );
    unsafe {
        let mut prev_read = 0;
        (gl.get_integerv)(GL_READ_FRAMEBUFFER_BINDING, &mut prev_read);
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, fbo);
        (gl.read_buffer)(0x8CE0);
        let step = (centres.len() / 6).max(1);
        for (k, p) in centres.iter().step_by(step).take(6).enumerate() {
            let c = mat_mul_vec(&vp, [p[0], p[1], p[2], 1.0]);
            if c[3] <= 0.0 {
                info_f!("Cruor CHECK   drop {}: world {:?} is BEHIND the camera (w {:.3})", k, [p[0].round(), p[1].round(), p[2].round()], c[3]);
                continue;
            }
            let (nx, ny, nz) = (c[0] / c[3], c[1] / c[3], c[2] / c[3]);
            let (px, py) = (((nx * 0.5 + 0.5) * w as f32) as i32, ((ny * 0.5 + 0.5) * h as f32) as i32);
            let dz = nz * 0.5 + 0.5;
            if px < 0 || py < 0 || px >= w || py >= h {
                info_f!(
                    "Cruor CHECK   drop {}: world {:?} lands OFF SCREEN at pixel ({}, {})",
                    k, [p[0].round(), p[1].round(), p[2].round()], px, py
                );
                continue;
            }
            let mut sd = [0.0f32; 1];
            gl_clear_errors(gl);
            (gl.read_pixels)(px, py, 1, 1, 0x1902 /* DEPTH_COMPONENT */, GL_FLOAT, sd.as_mut_ptr() as *mut c_void);
            let derr = (gl.get_error)();
            let mut col = [0u8; 4];
            (gl.read_pixels)(px, py, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, col.as_mut_ptr() as *mut c_void);
            info_f!(
                "Cruor CHECK   drop {}: world {:?} -> pixel ({}, {}); drop depth {:.6} vs scene depth {} -> {}; colour there now ({},{},{})",
                k, [p[0].round(), p[1].round(), p[2].round()], px, py, dz,
                if derr == 0 { format!("{:.6}", sd[0]) } else { format!("unreadable (0x{:X})", derr) },
                if derr != 0 { "?" } else if dz <= sd[0] { "should be VISIBLE" } else { "HIDDEN by the depth test" },
                col[0], col[1], col[2]
            );
        }
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, prev_read as u32);
        gl_clear_errors(gl);
    }
    // how many cameras the game drew each copy of the scene with this frame
    with_fs(|fs| {
        for (v, list) in fs.tile_votes.iter() {
            let mut counts: Vec<u32> = list.iter().map(|e| e.2).collect();
            counts.sort_unstable_by(|a, b| b.cmp(a));
            info_f!("Cruor CHECK   scene area {},{} {}x{}: draws per camera {:?}", v[0], v[1], v[2], v[3], counts);
        }
    });
    match GAME_GLOBAL.lock().ok().and_then(|g| *g) {
        Some((d, prog, binding, buf, start)) => {
            let f = |i: usize| d[i];
            info_f!("Cruor CHECK   GAME camera block (program {}, binding {}, buffer {} at {}):", prog, binding, buf, start);
            info_f!("Cruor CHECK     ViewPos  {:.1} {:.1} {:.1} {:.4}", f(0), f(1), f(2), f(3));
            info_f!("Cruor CHECK     ViewSiz  {:.3} {:.3}   ViewPrj {:.5} {:.5}", f(4), f(5), f(6), f(7));
            for row in 0..4 {
                let b = 8 + row * 4;
                info_f!("Cruor CHECK     CamMat[{}] {:.5} {:.5} {:.5} {:.5}", row, f(b), f(b + 1), f(b + 2), f(b + 3));
            }
            info_f!("Cruor CHECK     TimeDlt {:.4} {:.4}  ViewRcp {:.6} {:.6}  BufrPos {:.1} {:.1}", f(48), f(49), f(50), f(51), f(60), f(61));
        }
        None => info_f!("Cruor CHECK   GAME camera block: couldn't be read"),
    }
    info_f!(
        "Cruor CHECK   PLUGIN camera (from object matrices): row0 {:.5} {:.5} {:.5} {:.5} | row1 {:.5} {:.5} {:.5} {:.5} | row3 {:.5} {:.5} {:.5} {:.5}",
        vp[0], vp[4], vp[8], vp[12], vp[1], vp[5], vp[9], vp[13], vp[3], vp[7], vp[11], vp[15]
    );
    info_f!("Cruor CHECK ===== end =====");
}

/// The game has combined its copies into the full-size picture: draw the blood into it
/// (once, window resolution, tested against copy 0's saved depth).
unsafe fn draw_into_combined(sn: Snap) {
    let t = std::time::Instant::now();
    gpu_timer_begin(1);
    unsafe { draw_into_combined_inner(sn) };
    gpu_timer_end();
    perf_add(&PERF_INSCENE_NS, t);
}

unsafe fn draw_into_combined_inner(sn: Snap) {
    if sn.combined == 0 {
        if test_log_ok() {
            error_f!("Cruor TEST: the game didn't combine its copies into a picture I could see - NOT drawn");
        }
        return;
    }
    if !sn.depth_ok {
        return;
    }
    let (fbo, size) = match CMB.lock() {
        Ok(c) => (c.0, c.2),
        Err(_) => return,
    };
    let (w, h) = (sn.tile[2], sn.tile[3]);
    let mut prev_draw = 0;
    {
        let cell = match RENDERER.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        let r = match cell.0.as_ref() {
            Some(r) => r,
            None => return,
        };
        let gl = &r.gl;
        unsafe {
            gl_clear_errors_v(gl);
            // the combined picture must be the same size as a copy of the scene (its size and
            // our framebuffer's completeness are asked once per texture, and on verification frames)
            let cached = COMBINED_INFO.lock().ok().and_then(|c| *c).filter(|c| c.0 == sn.combined && c.1 == fbo && !gl_verify());
            let (cw, ch, known_ok) = if let Some(c) = cached { (c.2, c.3, true) } else {
                let (mut cw, mut ch, mut prev_tex) = (0, 0, 0);
                (gl.get_integerv)(GL_TEXTURE_BINDING_2D, &mut prev_tex);
                (gl.bind_texture)(GL_TEXTURE_2D, sn.combined);
                (gl.get_tex_level_parameteriv)(GL_TEXTURE_2D, 0, 0x1000, &mut cw);
                (gl.get_tex_level_parameteriv)(GL_TEXTURE_2D, 0, 0x1001, &mut ch);
                (gl.bind_texture)(GL_TEXTURE_2D, prev_tex as u32);
                (cw, ch, false)
            };
            if (cw, ch) != (w, h) || size != (w, h) {
                if test_log_ok() {
                    error_f!("Cruor TEST: combined picture is {}x{}, a scene copy is {}x{} - NOT drawn", cw, ch, w, h);
                }
                return;
            }
            prev_draw = CUR_DRAW_FBO.load(Ordering::Relaxed) as i32;
            (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, fbo);
            (gl.framebuffer_texture_2d)(GL_DRAW_FRAMEBUFFER, 0x8CE0, GL_TEXTURE_2D, sn.combined, 0);
            (gl.draw_buffer)(0x8CE0);
            let st = if known_ok { 0x8CD5 } else { (gl.check_framebuffer_status)(GL_DRAW_FRAMEBUFFER) };
            if st != 0x8CD5 {
                if test_log_ok() {
                    error_f!("Cruor TEST: can't draw into the combined picture (status 0x{:X}) - NOT drawn", st);
                }
                (gl.framebuffer_texture_2d)(GL_DRAW_FRAMEBUFFER, 0x8CE0, GL_TEXTURE_2D, 0, 0);
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, prev_draw as u32);
                return;
            }
            if !known_ok {
                if let Ok(mut c) = COMBINED_INFO.lock() {
                    *c = Some((sn.combined, fbo, cw, ch));
                }
            }
        }
    }
    if !CMB_LOGGED.swap(true, Ordering::Relaxed) {
        info_f!(
            "Cruor: flying blood is drawn once into the game's combined picture (texture {}, {}x{}) - the same on every supersampling setting",
            sn.combined, w, h
        );
    }
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace(format!("   ** BLOOD DRAWN HERE into the combined picture (texture {}, {}x{})", sn.combined, w, h));
    }
    CUR_COMBINED_TEX.store(sn.combined, Ordering::Relaxed);
    unsafe { in_scene_draw(fbo, &[([0, 0, w, h], sn.cam)]) };
    if let Ok(cell) = RENDERER.lock() {
        if let Some(r) = cell.0.as_ref() {
            let gl = &r.gl;
            unsafe {
                if DROP_CHECK.swap(false, Ordering::Relaxed) {
                    drop_check(gl, fbo, w, h, &sn.cam);
                }
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, fbo);
                (gl.framebuffer_texture_2d)(GL_DRAW_FRAMEBUFFER, 0x8CE0, GL_TEXTURE_2D, 0, 0);
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, prev_draw as u32);
                gl_clear_errors_v(gl);
            }
        }
    }
}

static REAL_COPY_TEX_SUB: AtomicUsize = AtomicUsize::new(0);
static REAL_COPY_TEX: AtomicUsize = AtomicUsize::new(0);
static REAL_GEN_MIPMAP: AtomicUsize = AtomicUsize::new(0);
static SNAPSHOT_LOGGED: AtomicBool = AtomicBool::new(false);

/// The game is about to take a snapshot of what it has drawn (copy into a texture, or
/// build mip levels). If that's the finished scene, draw the blood in first, so the
/// snapshot - which the final picture is made from - includes it.
/// Frames this 5-second period, and in how many the blood was drawn.
static FRAMES_SEEN: AtomicU32 = AtomicU32::new(0);
static FRAMES_DRAWN: AtomicU32 = AtomicU32::new(0);
static DRAWN_THIS_FRAME: AtomicBool = AtomicBool::new(false);
fn note_frame_drawn_stats() {
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    FRAMES_SEEN.fetch_add(1, Ordering::Relaxed);
    if DRAWN_THIS_FRAME.swap(false, Ordering::Relaxed) {
        FRAMES_DRAWN.fetch_add(1, Ordering::Relaxed);
    }
    if let Ok(mut t) = LAST.lock() {
        let now = std::time::Instant::now();
        let start = *t.get_or_insert(now);
        if (now - start).as_secs_f32() >= 5.0 {
            let (seen, drawn) = (FRAMES_SEEN.swap(0, Ordering::Relaxed), FRAMES_DRAWN.swap(0, Ordering::Relaxed));
            if !RELEASE && (!LAST_VERTS.lock().map(|v| v.is_empty()).unwrap_or(true) || drawn > 0) {
                info_f!("Cruor: flying blood drawn in {} of {} frames", drawn, seen);
            }
            cost_report();
            perf_report();
            *t = Some(now);
        }
    }
}

fn before_snapshot(what: &str) {
    let (scene, tiles) = match with_fs(|fs| {
        // the game combines / copies the buffer it drew the scene into (it has the scene's
        // camera draws) - no count, no threshold
        let ok = !fs.tiles.is_empty() && !fs.drawn_session;
        if ok {
            fs.drawn_session = true;
            Some((fs.cur_fbo, fs.tiles.clone()))
        } else {
            None
        }
    }) {
        Some(Some(x)) => x,
        _ => return,
    };
    if !SNAPSHOT_LOGGED.swap(true, Ordering::Relaxed) {
        log::debug!("Cruor TEST: the game snapshots its finished scene ({}) - blood is drawn right before it", what);
    }
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace_flush_draws();
        trace(format!(
            "   ** scene finished in buffer {} ({}) - copies: {}",
            scene,
            what,
            tiles.iter().map(|(v, _)| format!("{},{} {}x{}", v[0], v[1], v[2], v[3])).collect::<Vec<_>>().join(" | ")
        ));
    }
    // nothing in the air (and no marker test): no depth copy, no draw, nothing
    if !MARKER_ON.load(Ordering::Relaxed) && LAST_VERTS.lock().map(|v| v.is_empty()).unwrap_or(true) {
        return;
    }
    DRAWN_THIS_FRAME.store(true, Ordering::Relaxed);
    let t = std::time::Instant::now();
    if scene == 0 {
        // the scene is the window itself (no supersampling): draw straight into it
        CUR_COMBINED_TEX.store(0, Ordering::Relaxed);
        gpu_timer_begin(0);
        unsafe { in_scene_draw(scene, &tiles) };
        gpu_timer_end();
    } else {
        // supersampled: save the depth now, draw once into the combined picture later
        gpu_timer_begin(0);
        unsafe { capture_scene_depth(scene, &tiles) };
        gpu_timer_end();
    }
    perf_add(&PERF_INSCENE_NS, t);
}

unsafe extern "system" fn my_copy_tex_sub_image(target: u32, level: i32, xo: i32, yo: i32, x: i32, y: i32, w: i32, h: i32) {
    before_snapshot("copy into a texture");
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace_flush_draws();
        trace(format!("   >> SNAPSHOT: copy [{},{} {}x{}] into a texture (level {}, at {},{})", x, y, w, h, level, xo, yo));
    }
    let real: unsafe extern "system" fn(u32, i32, i32, i32, i32, i32, i32, i32) =
        unsafe { std::mem::transmute(REAL_COPY_TEX_SUB.load(Ordering::Relaxed)) };
    unsafe { real(target, level, xo, yo, x, y, w, h) }
}
unsafe extern "system" fn my_copy_tex_image(target: u32, level: i32, fmt: u32, x: i32, y: i32, w: i32, h: i32, border: i32) {
    before_snapshot("copy into a new texture");
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace_flush_draws();
        trace(format!("   >> SNAPSHOT: copy [{},{} {}x{}] into a new texture (level {}, format 0x{:X})", x, y, w, h, level, fmt));
    }
    let real: unsafe extern "system" fn(u32, i32, u32, i32, i32, i32, i32, i32) =
        unsafe { std::mem::transmute(REAL_COPY_TEX.load(Ordering::Relaxed)) };
    unsafe { real(target, level, fmt, x, y, w, h, border) }
}
unsafe extern "system" fn my_generate_mipmap(target: u32) {
    before_snapshot("building mip levels");
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace_flush_draws();
        trace("   >> SNAPSHOT: build mip levels of a texture".to_string());
    }
    let real: unsafe extern "system" fn(u32) = unsafe { std::mem::transmute(REAL_GEN_MIPMAP.load(Ordering::Relaxed)) };
    unsafe { real(target) }
}

static REAL_BIND_FB_EXT: AtomicUsize = AtomicUsize::new(0);
static REAL_BLIT_EXT: AtomicUsize = AtomicUsize::new(0);
static REAL_DRAW_ELEMENTS_INST: AtomicUsize = AtomicUsize::new(0);
static REAL_DRAW_ARRAYS_INST: AtomicUsize = AtomicUsize::new(0);
static EXT_SEEN: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn my_bind_framebuffer_ext(target: u32, fbo: u32) {
    if target == GL_FRAMEBUFFER || target == GL_READ_FRAMEBUFFER {
        CUR_READ_FBO.store(fbo, Ordering::Relaxed);
    }
    if target == GL_FRAMEBUFFER || target == GL_DRAW_FRAMEBUFFER {
        CUR_DRAW_FBO.store(fbo, Ordering::Relaxed);
    }
    if !EXT_SEEN.swap(true, Ordering::Relaxed) {
        test_f!("Cruor TEST: the game also switches buffers through glBindFramebufferEXT");
    }
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace(format!("   (EXT) switch target 0x{:X} to buffer {}", target, fbo));
    }
    let real: unsafe extern "system" fn(u32, u32) = unsafe { std::mem::transmute(REAL_BIND_FB_EXT.load(Ordering::Relaxed)) };
    unsafe { real(target, fbo) }
}
unsafe extern "system" fn my_blit_ext(sx0: i32, sy0: i32, sx1: i32, sy1: i32, dx0: i32, dy0: i32, dx1: i32, dy1: i32, mask: u32, filter: u32) {
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace(format!("   >> (EXT) COPY [{},{} - {},{}] -> [{},{} - {},{}] mask 0x{:X}", sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask));
    }
    let real: unsafe extern "system" fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32) =
        unsafe { std::mem::transmute(REAL_BLIT_EXT.load(Ordering::Relaxed)) };
    unsafe { real(sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask, filter) }
}
/// Draw calls per GL function this period (PERF): elements, range, instanced, arrays,
/// arrays instanced, multi-elements, multi-arrays.
static DRAW_KINDS: [AtomicU32; 7] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static REAL_MULTI_ELEMENTS: AtomicUsize = AtomicUsize::new(0);
static REAL_MULTI_ARRAYS: AtomicUsize = AtomicUsize::new(0);
unsafe extern "system" fn my_multi_draw_elements(mode: u32, counts: *const i32, ty: u32, indices: *const *const c_void, n: i32) {
    DRAW_KINDS[5].fetch_add(1, Ordering::Relaxed);
    visit_with(|v| v.draws += 1);
    note_post_draw();
    let first = if !counts.is_null() && n > 0 { unsafe { *counts } } else { 0 };
    before_draw(first, 0);
    let real: unsafe extern "system" fn(u32, *const i32, u32, *const *const c_void, i32) = unsafe { std::mem::transmute(REAL_MULTI_ELEMENTS.load(Ordering::Relaxed)) };
    unsafe { real(mode, counts, ty, indices, n) }
}
unsafe extern "system" fn my_multi_draw_arrays(mode: u32, firsts: *const i32, counts: *const i32, n: i32) {
    DRAW_KINDS[6].fetch_add(1, Ordering::Relaxed);
    visit_with(|v| v.draws += 1);
    note_post_draw();
    let first = if !counts.is_null() && n > 0 { unsafe { *counts } } else { 0 };
    before_draw(first, 0);
    let real: unsafe extern "system" fn(u32, *const i32, *const i32, i32) = unsafe { std::mem::transmute(REAL_MULTI_ARRAYS.load(Ordering::Relaxed)) };
    unsafe { real(mode, firsts, counts, n) }
}

unsafe extern "system" fn my_draw_elements_inst(mode: u32, count: i32, ty: u32, idx: *const c_void, n: i32) {
    DRAW_KINDS[2].fetch_add(1, Ordering::Relaxed);
    visit_with(|v| v.draws += 1);
    note_post_draw();
    before_draw(count, idx as usize);
    let real: unsafe extern "system" fn(u32, i32, u32, *const c_void, i32) = unsafe { std::mem::transmute(REAL_DRAW_ELEMENTS_INST.load(Ordering::Relaxed)) };
    unsafe { real(mode, count, ty, idx, n) }
}
unsafe extern "system" fn my_draw_arrays_inst(mode: u32, first: i32, count: i32, n: i32) {
    DRAW_KINDS[4].fetch_add(1, Ordering::Relaxed);
    visit_with(|v| v.draws += 1);
    note_post_draw();
    before_draw(count, first as usize);
    let real: unsafe extern "system" fn(u32, i32, i32, i32) = unsafe { std::mem::transmute(REAL_DRAW_ARRAYS_INST.load(Ordering::Relaxed)) };
    unsafe { real(mode, first, count, n) }
}

unsafe extern "system" fn my_draw_arrays(mode: u32, first: i32, count: i32) {
    DRAW_KINDS[3].fetch_add(1, Ordering::Relaxed);
    visit_with(|v| v.draws += 1);
    note_post_draw();
    before_draw(count, first as usize);
    let real: unsafe extern "system" fn(u32, i32, i32) = unsafe { std::mem::transmute(REAL_DRAW_ARRAYS.load(Ordering::Relaxed)) };
    unsafe { real(mode, first, count) }
}

/// After the game binds a framebuffer: remember which textures are attached to it.
unsafe fn note_fb_textures(fbo: u32) {
    if fbo == 0 {
        return;
    }
    unsafe {
        let q: unsafe extern "system" fn(u32, u32, u32, *mut i32) = std::mem::transmute(gl_resolve("glGetFramebufferAttachmentParameteriv"));
        for att in [0x8CE0u32, 0x8CE1, 0x8CE2, 0x8CE3, 0x8D00, 0x821A] {
            let (mut ty, mut name) = (0, 0);
            q(GL_DRAW_FRAMEBUFFER, att, 0x8CD0, &mut ty);
            if ty == 0x1702 {
                q(GL_DRAW_FRAMEBUFFER, att, 0x8CD1, &mut name);
                if let Ok(mut v) = FB_TEXTURES.lock() {
                    if !v.iter().any(|(t, f)| *t == name as u32 && *f == fbo) {
                        v.retain(|(t, _)| *t != name as u32);
                        v.push((name as u32, fbo));
                    }
                }
            }
        }
        let ge: unsafe extern "system" fn() -> u32 = std::mem::transmute(gl_resolve("glGetError"));
        for _ in 0..4 {
            if ge() == 0 {
                break;
            }
        }
    }
}

/// F11 / F12 (test build).
fn drop_check_poll() {
    static K7: AtomicBool = AtomicBool::new(false);
    let f7 = unsafe { GetAsyncKeyState(0x76) } as u16 & 0x8000 != 0;
    if f7 && !K7.swap(true, Ordering::Relaxed) {
        DROP_CHECK.store(true, Ordering::Relaxed);
        DRAW_COUNT.store(true, Ordering::Relaxed);
        ui_say("Cruor: flying-blood check - see the console", UI_WHITE);
    } else if !f7 {
        K7.store(false, Ordering::Relaxed);
    }
}

fn diag_keys_poll() {
    if RELEASE {
        return;
    }
    static K11: AtomicBool = AtomicBool::new(false);
    static K12: AtomicBool = AtomicBool::new(false);
    let f11 = unsafe { GetAsyncKeyState(0x7A) } as u16 & 0x8000 != 0;
    if f11 && !K11.swap(true, Ordering::Relaxed) {
        let v = !MARKER_ON.load(Ordering::Relaxed);
        MARKER_ON.store(v, Ordering::Relaxed);
        test_f!("Cruor TEST: marker {} (magenta square in each scene copy, drawn where the blood would be)", if v { "ON" } else { "OFF" });
    } else if !f11 {
        K11.store(false, Ordering::Relaxed);
    }
    static K9: AtomicBool = AtomicBool::new(false);
    let f9 = unsafe { GetAsyncKeyState(0x78) } as u16 & 0x8000 != 0;
    if f9 && !K9.swap(true, Ordering::Relaxed) {
        PROBE_REQ.store(true, Ordering::Relaxed);
    } else if !f9 {
        K9.store(false, Ordering::Relaxed);
    }
    let f12 = unsafe { GetAsyncKeyState(0x7B) } as u16 & 0x8000 != 0;
    if f12 && !K12.swap(true, Ordering::Relaxed) {
        TRACE_ARMED.store(true, Ordering::Relaxed);
        test_f!("Cruor TEST: tracing the next frame...");
    } else if !f12 {
        K12.store(false, Ordering::Relaxed);
    }
}

/// Called at the end of each frame: finish/print a trace, or start one.
fn trace_frame_boundary() {
    // this frame's window reads become "last frame's"
    if let Ok(mut w) = WINDOW_READS.lock() {
        w.1 = std::mem::take(&mut w.0);
    }
    if let Ok(mut w) = WINDOW_UNIT_TEX.lock() {
        w.1 = std::mem::take(&mut w.0);
    }
    if TRACE_ACTIVE.load(Ordering::Relaxed) {
        trace_flush_draws();
        TRACE_ACTIVE.store(false, Ordering::Relaxed);
        if let Ok(mut v) = VISIT.lock() {
            *v = None;
        }
        let lines = TRACE.lock().map(|mut t| std::mem::take(&mut *t)).unwrap_or_default();
        test_f!("Cruor TEST: ===== one frame, start to finish =====");
        for l in lines {
            info_f!("{}", l);
        }
        info_f!("   (frame shown on screen)");
        test_f!("Cruor TEST: ===== end of frame =====");
    }
    if TRACE_ARMED.swap(false, Ordering::Relaxed) {
        if let Ok(mut t) = TRACE.lock() {
            t.clear();
        }
        TRACE_DRAWS.store(0, Ordering::Relaxed);
        let cur = with_fs(|fs| fs.cur_fbo).unwrap_or(0);
        visit_start(cur);
        TRACE_ACTIVE.store(true, Ordering::Relaxed);
    }
}

/// Test-build error lines: at most 10 per 5 seconds, so a failure is visible but not a flood.
static TEST_LOG: Mutex<(u32, Option<std::time::Instant>)> = Mutex::new((0, None));
fn test_log_ok() -> bool {
    if let Ok(mut m) = UI_MSGS.lock().map_err(|_| ()).and_then(|g| if RELEASE { Err(()) } else { Ok(g) }) {
        if !m.iter().any(|(t, _, _)| t.starts_with("Cruor TEST")) {
            m.push(("Cruor TEST: a problem - see the console".to_string(), UI_RED, std::time::Instant::now()));
        }
    }
    let mut g = match TEST_LOG.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let now = std::time::Instant::now();
    let start = *g.1.get_or_insert(now);
    if (now - start).as_secs_f32() > 5.0 {
        *g = (0, Some(now));
    }
    g.0 += 1;
    g.0 <= 10
}

/// The scene buffer's colour layers: (layer n, texture name, mip level) for each attached texture.
unsafe fn scene_layers(gl: &Gl) -> Vec<(u32, u32, i32)> {
    let mut out = Vec::new();
    unsafe {
        for n in 0..8u32 {
            let (mut ty, mut name, mut lvl) = (0, 0, 0);
            (gl.get_framebuffer_attachment_parameteriv)(GL_DRAW_FRAMEBUFFER, 0x8CE0 + n, 0x8CD0, &mut ty);
            if ty == 0 {
                continue;
            }
            (gl.get_framebuffer_attachment_parameteriv)(GL_DRAW_FRAMEBUFFER, 0x8CE0 + n, 0x8CD1, &mut name);
            if ty == 0x1702 {
                (gl.get_framebuffer_attachment_parameteriv)(GL_DRAW_FRAMEBUFFER, 0x8CE0 + n, 0x8CD2, &mut lvl);
            }
            out.push((n, name as u32, if ty == 0x1702 { lvl } else { -1 }));
        }
        gl_clear_errors(gl);
    }
    out
}
/// Which layers draws currently go to (GL_DRAW_BUFFERi).
unsafe fn draw_buffers_now(gl: &Gl) -> Vec<u32> {
    let mut v = Vec::new();
    unsafe {
        let mut maxb = 1;
        (gl.get_integerv)(0x8824, &mut maxb); // MAX_DRAW_BUFFERS
        for i in 0..maxb.clamp(1, 8) as u32 {
            let mut b = 0;
            (gl.get_integerv)(0x8825 + i, &mut b);
            v.push(b as u32);
        }
    }
    v
}
fn buf_name(b: u32) -> String {
    match b {
        0 => "none".into(),
        0x0405 => "back".into(),
        x if (0x8CE0..0x8CF0).contains(&x) => format!("layer {}", x - 0x8CE0),
        x => format!("0x{:X}", x),
    }
}
static PICK_LOGGED: AtomicU32 = AtomicU32::new(u32::MAX);

/// Put the game's draw-buffer setting back. The window takes a single glDrawBuffer;
/// framebuffer objects take the full glDrawBuffers list.
unsafe fn restore_draw_buffers(gl: &Gl, fbo: u32, prev: &[u32]) {
    unsafe {
        if fbo == 0 {
            (gl.draw_buffer)(prev.first().copied().unwrap_or(GL_BACK));
        } else {
            // trim trailing "none"s (some drivers dislike long lists)
            let mut n = prev.len();
            while n > 1 && prev[n - 1] == 0 {
                n -= 1;
            }
            (gl.draw_buffers)(n as i32, prev.as_ptr());
        }
    }
}

/// MARKER PIXEL CHECK: where the magenta goes. (buffer, x, y) of the marker centre in the
/// scene, and the colour read right after painting it.
static PIX_SCENE: Mutex<Option<(u32, i32, i32, [u8; 4], f32, f32)>> = Mutex::new(None);
static PIX_LAST_LOG: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static PIX_B4_LOGGED: AtomicBool = AtomicBool::new(false);

/// Read one pixel of a framebuffer's colour (restores the read binding).
unsafe fn read_pixel(gl: &Gl, fbo: u32, x: i32, y: i32) -> Result<[u8; 4], u32> {
    unsafe {
        let (mut prev_fb, mut prev_rb) = (0, 0);
        (gl.get_integerv)(GL_READ_FRAMEBUFFER_BINDING, &mut prev_fb);
        gl_clear_errors(gl);
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, fbo);
        (gl.get_integerv)(GL_READ_BUFFER, &mut prev_rb);
        (gl.read_buffer)(if fbo == 0 { GL_BACK } else { 0x8CE0 });
        let mut px = [0u8; 4];
        (gl.read_pixels)(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        let err = (gl.get_error)();
        (gl.read_buffer)(prev_rb as u32);
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, prev_fb as u32);
        gl_clear_errors(gl);
        if err == 0 { Ok(px) } else { Err(err) }
    }
}
fn px_str(p: Result<[u8; 4], u32>) -> String {
    match p {
        Ok(c) => {
            let magenta = c[0] > 200 && c[1] < 60 && c[2] > 200;
            format!("({:3},{:3},{:3}){}", c[0], c[1], c[2], if magenta { " MAGENTA" } else { "" })
        }
        Err(e) => format!("(couldn't read: error 0x{:X})", e),
    }
}

static PROBE_FBO: AtomicU32 = AtomicU32::new(0);
/// Read the marker's spot (as a fraction of the picture) in a texture: size + colour.
unsafe fn texture_pixel(gl: &Gl, tex: u32, fx: f32, fy: f32) -> String {
    unsafe {
        let (mut prev_fb, mut prev_tex, mut w, mut h, mut fmt) = (0, 0, 0, 0, 0);
        (gl.get_integerv)(GL_READ_FRAMEBUFFER_BINDING, &mut prev_fb);
        (gl.get_integerv)(GL_TEXTURE_BINDING_2D, &mut prev_tex);
        gl_clear_errors(gl);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.get_tex_level_parameteriv)(GL_TEXTURE_2D, 0, 0x1000, &mut w);
        (gl.get_tex_level_parameteriv)(GL_TEXTURE_2D, 0, 0x1001, &mut h);
        (gl.get_tex_level_parameteriv)(GL_TEXTURE_2D, 0, 0x1003, &mut fmt); // INTERNAL_FORMAT
        (gl.bind_texture)(GL_TEXTURE_2D, prev_tex as u32);
        if (gl.get_error)() != 0 || w <= 0 || h <= 0 {
            gl_clear_errors(gl);
            return "(not a 2D image)".into();
        }
        let mut fbo = PROBE_FBO.load(Ordering::Relaxed);
        if fbo == 0 {
            (gl.gen_framebuffers)(1, &mut fbo);
            PROBE_FBO.store(fbo, Ordering::Relaxed);
        }
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, fbo);
        (gl.framebuffer_texture_2d)(GL_READ_FRAMEBUFFER, 0x8CE0, GL_TEXTURE_2D, tex, 0);
        (gl.read_buffer)(0x8CE0);
        let st = (gl.check_framebuffer_status)(GL_READ_FRAMEBUFFER);
        let mut px = [0u8; 4];
        let mut err = 0;
        if st == 0x8CD5 {
            (gl.read_pixels)((fx * w as f32) as i32, (fy * h as f32) as i32, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
            err = (gl.get_error)();
        }
        (gl.framebuffer_texture_2d)(GL_READ_FRAMEBUFFER, 0x8CE0, GL_TEXTURE_2D, 0, 0);
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, prev_fb as u32);
        gl_clear_errors(gl);
        if st != 0x8CD5 {
            return format!("{}x{} fmt 0x{:X} (can't read: 0x{:X})", w, h, fmt, st);
        }
        format!("{}x{} fmt 0x{:X} {}", w, h, fmt, px_str(if err == 0 { Ok(px) } else { Err(err) }))
    }
}

/// At the end of the frame (before it's shown): read the marker's spot in the scene
/// buffer, buffer 4 and the window, and log all four readings together (once a second).
unsafe fn marker_pixel_report(gl: &Gl) {
    if !MARKER_ON.load(Ordering::Relaxed) {
        return;
    }
    let (fbo, x, y, after, fx, fy) = match PIX_SCENE.lock().ok().and_then(|p| *p) {
        Some(v) => v,
        None => return,
    };
    if let Ok(mut t) = PIX_LAST_LOG.lock() {
        if t.map(|t| t.elapsed().as_secs_f32() < 1.0).unwrap_or(false) {
            return;
        }
        *t = Some(std::time::Instant::now());
    }
    unsafe {
        let scene_end = read_pixel(gl, fbo, x, y);
        // buffer 4 and the window: the same spot of the picture (a copy of the scene
        // covers the whole picture), scaled to each one's size
        let mut rect = [0i32; 4];
        let hwnd = WindowFromDC((gl.get_current_dc)());
        let (ww, wh) = if !hwnd.is_null() && GetClientRect(hwnd, &mut rect) != 0 { (rect[2] - rect[0], rect[3] - rect[1]) } else { (0, 0) };
        // buffer 4's image size
        let (mut b4w, mut b4h) = (0, 0);
        {
            let (mut prev_fb, mut name, mut ty, mut lvl, mut prev_tex) = (0, 0, 0, 0, 0);
            (gl.get_integerv)(GL_READ_FRAMEBUFFER_BINDING, &mut prev_fb);
            (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, 4);
            (gl.get_framebuffer_attachment_parameteriv)(GL_READ_FRAMEBUFFER, 0x8CE0, 0x8CD0, &mut ty);
            (gl.get_framebuffer_attachment_parameteriv)(GL_READ_FRAMEBUFFER, 0x8CE0, 0x8CD1, &mut name);
            (gl.get_framebuffer_attachment_parameteriv)(GL_READ_FRAMEBUFFER, 0x8CE0, 0x8CD2, &mut lvl);
            (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, prev_fb as u32);
            if ty == 0x1702 {
                (gl.get_integerv)(GL_TEXTURE_BINDING_2D, &mut prev_tex);
                (gl.bind_texture)(GL_TEXTURE_2D, name as u32);
                (gl.get_tex_level_parameteriv)(GL_TEXTURE_2D, lvl, 0x1000, &mut b4w); // TEXTURE_WIDTH
                (gl.get_tex_level_parameteriv)(GL_TEXTURE_2D, lvl, 0x1001, &mut b4h); // TEXTURE_HEIGHT
                (gl.bind_texture)(GL_TEXTURE_2D, prev_tex as u32);
            }
            gl_clear_errors(gl);
            if PIX_B4_LOGGED.swap(true, Ordering::Relaxed) == false {
                test_f!("Cruor TEST: buffer 4 image = texture {} level {} ({}x{}); window {}x{}", name, lvl, b4w, b4h, ww, wh);
            }
        }
        let b4 = read_pixel(gl, 4, (fx * b4w as f32) as i32, (fy * b4h as f32) as i32);
        let win = read_pixel(gl, 0, (fx * ww as f32) as i32, (fy * wh as f32) as i32);
        // every texture the window's draws used (last frame), plus the scene's own image
        let mut texs: Vec<u32> = WINDOW_UNIT_TEX.lock().map(|w| w.1.clone()).unwrap_or_default();
        for t in WINDOW_READS.lock().map(|w| w.1.clone()).unwrap_or_default() {
            if !texs.contains(&t) {
                texs.push(t);
            }
        }
        let mut parts = Vec::new();
        for t in texs.iter().take(12) {
            parts.push(format!("texture {} {}", t, texture_pixel(gl, *t, fx, fy)));
        }
        info_f!("Cruor TEST textures the window used, at the marker spot: {}", parts.join(" | "));
        info_f!(
            "Cruor TEST pixels at the marker: buffer {} right after painting {} | buffer {} end of frame {} | buffer 4 end of frame {} | window end of frame {}",
            fbo, px_str(Ok(after)), fbo, px_str(scene_end), px_str(b4), px_str(win)
        );
    }
}

/// Clear pending OpenGL errors.
/// VERIFICATION FRAMES. Asking the driver anything (glGetError, format/size/status queries)
/// can make a threaded GL driver stop and sync with its worker thread - a stall that grows
/// with how busy the card is. The per-frame drawing therefore only checks for errors and
/// re-asks things that don't change (buffer formats, sizes, completeness) on these frames:
/// the first 120 after the renderer starts or any error, then 1 frame in 512.
static GL_VERIFY_LEFT: AtomicU32 = AtomicU32::new(120);
fn gl_verify() -> bool {
    GL_VERIFY_LEFT.load(Ordering::Relaxed) > 0 || FRAME.load(Ordering::Relaxed) % 512 == 0
}
fn gl_verify_again() {
    GL_VERIFY_LEFT.store(120, Ordering::Relaxed);
}
fn gl_verify_tick() {
    let _ = GL_VERIFY_LEFT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| if v > 0 { Some(v - 1) } else { None });
}
/// Per-frame error clear: only on verification frames.
unsafe fn gl_clear_errors_v(gl: &Gl) {
    if gl_verify() {
        unsafe { gl_clear_errors(gl) };
    }
}
/// Per-frame error check: only on verification frames (0 = no error); an error starts
/// another run of verification frames.
unsafe fn gl_err_v(gl: &Gl) -> u32 {
    if !gl_verify() {
        return 0;
    }
    let e = unsafe { (gl.get_error)() };
    if e != 0 {
        gl_verify_again();
    }
    e
}
/// The game's current read framebuffer, as its (hooked) glBindFramebuffer calls set it.
static CUR_READ_FBO: AtomicU32 = AtomicU32::new(0);
/// The texture the combined-picture pass has attached to the plugin's framebuffer right now
/// (0 = drawing straight into the window) - part of the scene-description cache key.
static CUR_COMBINED_TEX: AtomicU32 = AtomicU32::new(0);
/// Cached answers that don't change between frames (asked again on verification frames).
static DEPTH_INFO: Mutex<Option<(u32, i32, i32, u32, i32, bool)>> = Mutex::new(None); // scene, w, h -> scene_tex, fmt, is_ds
static COMBINED_INFO: Mutex<Option<(u32, u32, i32, i32)>> = Mutex::new(None); // combined tex, fbo, w, h (verified complete)
static READBUF_CACHE: Mutex<Option<(u32, i32)>> = Mutex::new(None);
static DESC_CACHE: Mutex<Option<((u32, u32), SceneInfo, Vec<(u32, u32, i32)>, Vec<u32>)>> = Mutex::new(None);

unsafe fn gl_clear_errors(gl: &Gl) {
    for _ in 0..16 {
        if unsafe { (gl.get_error)() } == 0 {
            break;
        }
    }
}

/// Draw the flying blood into the game's scene buffer, right as the scene is finished:
/// into every copy the game drew (supersampling), each with that copy's own camera.
/// Every step is checked on its own; a failing background copy only costs the
/// see-through effect, a failing draw falls back to the overlay.
unsafe fn in_scene_draw(scene_fbo: u32, tiles: &[([i32; 4], [f32; 32])]) {
    if tiles.is_empty() {
        return;
    }
    let frame = FRAME.load(Ordering::Relaxed);
    let verts = match LAST_VERTS.lock() {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut cell = match RENDERER.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    if let Some(r) = cell.0.as_ref() {
        if !unsafe { renderer_healthy(r) } {
            return; // rebuilt at the end of the frame
        }
    }
    let r = match cell.0.as_mut() {
        Some(r) => r,
        None => return,
    };
    unsafe {
        let gl = &r.gl;
        // --- what is this scene buffer? (asked once per buffer + attached picture, and on
        // verification frames - its formats and colour layers don't change between frames) ---
        let key = (scene_fbo, CUR_COMBINED_TEX.load(Ordering::Relaxed));
        let cached_desc = DESC_CACHE.lock().ok().and_then(|c| c.clone()).filter(|c| c.0 == key && !gl_verify());
        let (d_si, layers, prev_bufs) = match cached_desc {
            Some(c) => (c.1, c.2, c.3),
            None => {
                let d = (describe_scene(gl, scene_fbo), scene_layers(gl), draw_buffers_now(gl));
                if let Ok(mut c) = DESC_CACHE.lock() {
                    *c = Some((key, d.0.clone(), d.1.clone(), d.2.clone()));
                }
                d
            }
        };
        let info = {
            let mut g = SCENE_INFO.lock().unwrap();
            let si = d_si.clone();
            let changed = g.as_ref().map(|old| old.fbo != si.fbo || old.desc != si.desc).unwrap_or(true);
            if changed {
                r.sbg_size = (0, 0); // (re)make the background copy for it
                if let Ok(mut l) = SCENE_LOGGED.lock() {
                    l.retain(|f| *f != scene_fbo); // log the new description
                }
            }
            *g = Some(si.clone());
            si
        };
        let first_time = {
            let mut l = SCENE_LOGGED.lock().unwrap();
            if l.contains(&scene_fbo) { false } else { l.push(scene_fbo); true }
        };
        if first_time {
            info_f!("Cruor: game scene is {} ({} cop{} per frame)", info.desc, tiles.len(), if tiles.len() == 1 { "y" } else { "ies" });
        }
        // Its colour layers, which are switched on right now, and which one the final
        // picture reads (the window pass reads that texture).
        let window_reads = WINDOW_READS.lock().map(|w| w.1.clone()).unwrap_or_default();
        let picture = layers.iter().find(|(_, name, _)| window_reads.contains(name)).map(|l| l.0);
        let target = picture.unwrap_or(0);
        if PICK_LOGGED.swap(target, Ordering::Relaxed) != target || first_time {
            log::debug!(
                "Cruor TEST: scene layers {}; switched on now: [{}]; window's final pass reads textures {:?}; blood goes to layer {}{}",
                layers.iter().map(|(n, name, l)| format!("{}=texture {} (level {})", n, name, l)).collect::<Vec<_>>().join(", "),
                prev_bufs.iter().map(|b| buf_name(*b)).collect::<Vec<_>>().join(", "),
                window_reads,
                target,
                if picture.is_some() { " (the one the final picture reads)" } else { " (no match found - layer 0)" }
            );
        }
        if TRACE_ACTIVE.load(Ordering::Relaxed) {
            trace(format!(
                "   scene layers {:?}; on now [{}]; window reads {:?}; blood -> layer {}",
                layers, prev_bufs.iter().map(|b| buf_name(*b)).collect::<Vec<_>>().join(", "), window_reads, target
            ));
        }
        if !info.has_depth {
            if first_time {
                error_f!("Cruor TEST: that scene has no depth - flying blood NOT drawn");
            }
            return;
        }
        // (counts as done even with nothing to draw, so the overlay stays off)
        IN_SCENE_FRAME.store(frame.max(1), Ordering::Relaxed);
        let marker_test = MARKER_ON.load(Ordering::Relaxed);
        if (verts.is_empty() && !marker_test) || !BLOOD_ON.load(Ordering::Relaxed) {
            return;
        }

        // --- save the game's state ---
        let mut prev_prog = 0;
        let mut prev_vao = 0;
        let mut prev_buf = 0;
        let mut prev_read_fb = 0;
        let mut prev_draw_fb = 0;
        let mut prev_vp = [0i32; 4];
        let mut prev_mask = 0;
        let mut prev_depth_func = 0;
        let mut prev_active = 0;
        let mut prev_tex = 0;
        let mut bs = [0i32; 4];
        let mut be = [0i32; 2];
        let mut prev_cmask = [1u8; 4];
        // (program, VAO, framebuffers and texture unit: as the game's hooked calls set them -
        // no need to ask the driver)
        prev_prog = CURRENT_PROGRAM.load(Ordering::Relaxed) as i32;
        prev_vao = CUR_VAO.load(Ordering::Relaxed) as i32;
        (gl.get_integerv)(GL_ARRAY_BUFFER_BINDING, &mut prev_buf);
        prev_read_fb = CUR_READ_FBO.load(Ordering::Relaxed) as i32;
        prev_draw_fb = CUR_DRAW_FBO.load(Ordering::Relaxed) as i32;
        (gl.get_integerv)(GL_VIEWPORT, prev_vp.as_mut_ptr());
        (gl.get_integerv)(GL_DEPTH_WRITEMASK, &mut prev_mask);
        (gl.get_integerv)(GL_DEPTH_FUNC, &mut prev_depth_func);
        prev_active = (0x84C0 + CUR_UNIT.load(Ordering::Relaxed)) as i32;
        (gl.active_texture)(GL_TEXTURE0 + BG_UNIT);
        (gl.get_integerv)(GL_TEXTURE_BINDING_2D, &mut prev_tex);
        (gl.get_integerv)(GL_BLEND_SRC_RGB, &mut bs[0]);
        (gl.get_integerv)(GL_BLEND_DST_RGB, &mut bs[1]);
        (gl.get_integerv)(GL_BLEND_SRC_ALPHA, &mut bs[2]);
        (gl.get_integerv)(GL_BLEND_DST_ALPHA, &mut bs[3]);
        (gl.get_integerv)(0x8009, &mut be[0]); // BLEND_EQUATION_RGB
        (gl.get_integerv)(0x883D, &mut be[1]); // BLEND_EQUATION_ALPHA
        (gl.get_booleanv)(GL_COLOR_WRITEMASK, prev_cmask.as_mut_ptr());
        let caps = [GL_BLEND, GL_DEPTH_TEST, GL_CULL_FACE, GL_SCISSOR_TEST, GL_STENCIL_TEST, 0x809E /* SAMPLE_ALPHA_TO_COVERAGE */];
        let was: Vec<u8> = caps.iter().map(|&c| unsafe { (gl.is_enabled)(c) }).collect();
        gl_clear_errors_v(gl);

        // --- the background copy: our own texture in the scene's exact colour format ---
        // (a resolve-blit: works whether the scene is multisampled or not)
        let big = tiles.iter().max_by_key(|(v, _)| v[2] * v[3]).map(|(v, _)| *v).unwrap_or([0; 4]);
        let (tw, th) = (big[2], big[3]);
        if r.sbg_size != (tw, th) || r.sbg_fmt != info.color_fmt {
            r.sbg_ok = false;
            if info.color_fmt != 0 && tw > 0 && th > 0 {
                if r.sbg_tex == 0 {
                    (gl.gen_textures)(1, &mut r.sbg_tex);
                    (gl.gen_framebuffers)(1, &mut r.sbg_fbo);
                }
                (gl.bind_texture)(GL_TEXTURE_2D, r.sbg_tex);
                (gl.tex_image_2d)(GL_TEXTURE_2D, 0, info.color_fmt, tw, th, 0, GL_RGBA, GL_FLOAT, std::ptr::null());
                (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
                (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
                (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, r.sbg_fbo);
                (gl.framebuffer_texture_2d)(GL_DRAW_FRAMEBUFFER, 0x8CE0, GL_TEXTURE_2D, r.sbg_tex, 0);
                (gl.draw_buffer)(0x8CE0);
                let status = (gl.check_framebuffer_status)(GL_DRAW_FRAMEBUFFER);
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, scene_fbo);
                let err = (gl.get_error)();
                r.sbg_ok = status == 0x8CD5 && err == 0;
                if !r.sbg_ok {
                    error_f!(
                        "Cruor TEST: couldn't make the background copy for {} (format 0x{:X}, status 0x{:X}, error 0x{:X}) - flying blood NOT drawn",
                        info.desc, info.color_fmt, status, err
                    );
                }
            } else if info.color_fmt == 0 {
                error_f!("Cruor TEST: unknown colour format for {} - flying blood NOT drawn", info.desc);
            }
            r.sbg_size = (tw, th);
            r.sbg_fmt = info.color_fmt;
        }

        let mut drawn = 0;
        // Only the full-size copies of the scene (the game also draws small things into
        // this buffer with its scene shaders; those aren't copies of the scene).
        let full = tiles.iter().map(|(v, _)| v[2] * v[3]).max().unwrap_or(0);
        let marker = MARKER_ON.load(Ordering::Relaxed);
        for (sv, cam) in tiles.iter() {
            let sv = *sv;
            if sv[2] <= 0 || sv[3] <= 0 || sv[2] * sv[3] < full {
                continue;
            }
            if marker {
                // MARKER TEST: in the middle of this copy, a stripe per colour layer:
                // layer 0 magenta, 1 green, 2 cyan, 3 yellow (whichever shows = the picture).
                let mut prev_cc = [0.0f32; 4];
                let mut prev_sb = [0i32; 4];
                (gl.get_floatv)(0x0C22, prev_cc.as_mut_ptr()); // COLOR_CLEAR_VALUE
                (gl.get_integerv)(0x0C10, prev_sb.as_mut_ptr()); // SCISSOR_BOX
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, scene_fbo);
                (gl.color_mask)(1, 1, 1, 1);
                (gl.enable)(GL_SCISSOR_TEST);
                let colours = [[1.0f32, 0.0, 1.0], [0.0, 1.0, 0.0], [0.0, 1.0, 1.0], [1.0, 1.0, 0.0]];
                // (the window has no numbered layers: paint its one colour image)
                let marker_layers: Vec<u32> = if scene_fbo == 0 {
                    vec![u32::MAX]
                } else {
                    layers.iter().map(|l| l.0).collect()
                };
                for (k, n) in marker_layers.iter().enumerate().take(4) {
                    let c = colours[k.min(3)];
                    (gl.draw_buffer)(if *n == u32::MAX { prev_bufs.first().copied().unwrap_or(GL_BACK) } else { 0x8CE0 + *n });
                    let stripe = sv[2] / 4 / 4;
                    (gl.scissor)(sv[0] + sv[2] * 3 / 8 + stripe * k as i32, sv[1] + sv[3] * 3 / 8, stripe, sv[3] / 4);
                    (gl.clear_color)(c[0], c[1], c[2], 1.0);
                    (gl.clear)(0x4000);
                }
                restore_draw_buffers(gl, scene_fbo, &prev_bufs);
                (gl.clear_color)(prev_cc[0], prev_cc[1], prev_cc[2], prev_cc[3]);
                (gl.scissor)(prev_sb[0], prev_sb[1], prev_sb[2], prev_sb[3]);
                if drawn == 0 {
                    // the magenta (layer 0) stripe's centre, read back right away
                    let stripe = sv[2] / 4 / 4;
                    let (px, py) = (sv[0] + sv[2] * 3 / 8 + stripe / 2, sv[1] + sv[3] / 2);
                    (gl.disable)(GL_SCISSOR_TEST);
                    let after = read_pixel(gl, scene_fbo, px, py).unwrap_or([0; 4]);
                    (gl.enable)(GL_SCISSOR_TEST);
                    let fx = (px - sv[0]) as f32 / sv[2] as f32;
                    let fy = (py - sv[1]) as f32 / sv[3] as f32;
                    if let Ok(mut p) = PIX_SCENE.lock() {
                        *p = Some((scene_fbo, px, py, after, fx, fy));
                    }
                }
                let err = (gl.get_error)();
                if err != 0 && test_log_ok() {
                    error_f!("Cruor TEST: marker failed (error 0x{:X})", err);
                }
                drawn += 1;
                continue;
            }
            let vp = match tile_view_proj(cam) {
                Some(v) => v,
                None => continue,
            };
            let inv_vp = match mat_inverse(&vp) {
                Some(m) => m,
                None => continue,
            };
            let (sw, sh) = (sv[2], sv[3]);

            // 1) background copy of this copy of the scene (required - no drawing without it)
            let mut have_bg = false;
            if !r.sbg_ok || (sw, sh) != r.sbg_size {
                if test_log_ok() {
                    error_f!("Cruor TEST: no background copy for copy {}x{} (made for {}x{}) - NOT drawn", sw, sh, r.sbg_size.0, r.sbg_size.1);
                }
                continue;
            }
            {
                gl_clear_errors_v(gl);
                (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, scene_fbo);
                // (this buffer's read setting only changes here, and is put back: asked once per
                // buffer, and on verification frames)
                let cached_rb = READBUF_CACHE.lock().ok().and_then(|c| *c).filter(|c| c.0 == scene_fbo && !gl_verify());
                let scene_read_buf = match cached_rb {
                    Some(c) => c.1,
                    None => {
                        let mut v = 0;
                        (gl.get_integerv)(GL_READ_BUFFER, &mut v);
                        if let Ok(mut c) = READBUF_CACHE.lock() {
                            *c = Some((scene_fbo, v));
                        }
                        v
                    }
                };
                (gl.read_buffer)(if scene_fbo == 0 { GL_BACK } else { 0x8CE0 });
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, r.sbg_fbo);
                (gl.blit_framebuffer)(sv[0], sv[1], sv[0] + sw, sv[1] + sh, 0, 0, sw, sh, 0x4000 /* COLOR */, GL_NEAREST);
                (gl.read_buffer)(scene_read_buf as u32);
                (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, scene_fbo);
                let err = gl_err_v(gl);
                if err == 0 {
                    have_bg = true;
                } else if test_log_ok() {
                    error_f!("Cruor TEST: background copy failed (error 0x{:X}) on {} - NOT drawn", err, info.desc);
                }
            }
            if !have_bg {
                continue;
            }

            // 2) draw into the scene, tested against its own depth
            gl_clear_errors_v(gl);
            (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, scene_fbo);
            (gl.viewport)(sv[0], sv[1], sw, sh);
            (gl.disable)(GL_SCISSOR_TEST);
            (gl.disable)(GL_STENCIL_TEST);
            (gl.disable)(GL_CULL_FACE);
            (gl.disable)(0x809E);
            (gl.color_mask)(1, 1, 1, 1);
            (gl.enable)(GL_BLEND);
            (gl.blend_equation_separate)(0x8006, 0x8006); // FUNC_ADD
            (gl.blend_func_separate)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ZERO, GL_ONE);
            (gl.enable)(GL_DEPTH_TEST);
            (gl.depth_func)(GL_LEQUAL);
            (gl.depth_mask)(0);
            (gl.active_texture)(GL_TEXTURE0 + BG_UNIT);
            (gl.bind_texture)(GL_TEXTURE_2D, if have_bg { r.sbg_tex } else { 0 });
            let right = norm3([vp[0], vp[4], vp[8]]);
            let up = norm3([vp[1], vp[5], vp[9]]);
            let fwd = norm3([vp[3], vp[7], vp[11]]);
            let e = mat_mul_vec(&inv_vp, [0.0, 0.0, 1.0, 0.0]);
            let eye = if e[3].abs() > 1e-6 { [e[0] / e[3], e[1] / e[3], e[2] / e[3]] } else { [0.0; 3] };
            (gl.use_program)(r.program);
            (gl.uniform_matrix4fv)(r.u_view_proj, 1, 0, vp.as_ptr());
            (gl.uniform3f)(r.u_right, right[0], right[1], right[2]);
            (gl.uniform3f)(r.u_up, up[0], up[1], up[2]);
            (gl.uniform3f)(r.u_fwd, fwd[0], fwd[1], fwd[2]);
            (gl.uniform3f)(r.u_eye, eye[0], eye[1], eye[2]);
            (gl.uniform2f)(r.u_viewport, sw as f32, sh as f32);
            (gl.uniform2f)(r.u_vp_origin, sv[0] as f32, sv[1] as f32);
            (gl.uniform1f)(r.u_have_bg, if have_bg { 1.0 } else { 0.0 });
            (gl.uniform1f)(r.u_scene_space, 1.0);
            (gl.uniform1f)(r.u_drop, (0.9 / tv(T_DARK).max(0.2)).clamp(0.2, 3.0));
            // sRGB scene images are converted by the card when read; plain ones we convert
            let srgb_scene = matches!(info.color_fmt, 0x8C41 | 0x8C43);
            (gl.uniform1f)(r.u_decode_bg, if srgb_scene { 0.0 } else { 1.0 });
            (gl.uniform1f)(r.u_scale, VISUAL_SCALE);
            (gl.uniform1f)(r.u_stretch, STRETCH);
            (gl.uniform1i)(r.u_bg, BG_UNIT as i32);
            (gl.bind_vertex_array)(r.vao);
            (gl.bind_buffer)(GL_ARRAY_BUFFER, r.vbo);
            (gl.buffer_data)(GL_ARRAY_BUFFER, (verts.len() * 4) as isize, verts.as_ptr() as *const c_void, GL_STREAM_DRAW);
            (gl.draw_buffer)(if scene_fbo == 0 { prev_bufs.first().copied().unwrap_or(GL_BACK) } else { 0x8CE0 + target });
            let counting = DRAW_COUNT.swap(false, Ordering::Relaxed);
            let mut q = [0u32; 2];
            if counting {
                if let Ok(mut qs) = QUERIES.lock() {
                    if qs[0] == 0 {
                        (gl.gen_queries)(2, qs.as_mut_ptr());
                    }
                    q = *qs;
                }
                (gl.begin_query)(0x8914 /* SAMPLES_PASSED */, q[0]);
            }
            (gl.draw_arrays)(GL_TRIANGLES, 0, (verts.len() / 9) as i32);
            if counting {
                (gl.end_query)(0x8914);
                // the same drops again, invisibly (no colour, no depth writes) and
                // without the depth test: how many pixels they cover at all
                (gl.color_mask)(0, 0, 0, 0);
                (gl.disable)(GL_DEPTH_TEST);
                (gl.begin_query)(0x8914, q[1]);
                (gl.draw_arrays)(GL_TRIANGLES, 0, (verts.len() / 9) as i32);
                (gl.end_query)(0x8914);
                (gl.enable)(GL_DEPTH_TEST);
                (gl.color_mask)(1, 1, 1, 1);
                let (mut written, mut covered) = (0u32, 0u32);
                (gl.get_query_objectuiv)(q[0], 0x8866 /* QUERY_RESULT */, &mut written);
                (gl.get_query_objectuiv)(q[1], 0x8866, &mut covered);
                info_f!(
                    "Cruor CHECK   GPU count: {} flying drops drawn into {}x{}; pixels they cover: {}; pixels actually written (passed the depth test): {}",
                    verts.len() / 54, sw, sh, covered, written
                );
            }
            restore_draw_buffers(gl, scene_fbo, &prev_bufs);
            let err = gl_err_v(gl);
            if err != 0 {
                if test_log_ok() {
                    error_f!("Cruor TEST: drawing inside the scene failed (error 0x{:X}) on {}", err, info.desc);
                }
                continue;
            }
            drawn += 1;
        }
        if drawn > 0 && !IN_SCENE_LOGGED.swap(true, Ordering::Relaxed) {
            info_f!(
                "Cruor: flying blood is drawn inside the game's scene ({} cop{}, see-through {})",
                drawn, if drawn == 1 { "y" } else { "ies" }, if r.sbg_ok { "on" } else { "off" }
            );
        }

        // --- restore ---
        (gl.bind_vertex_array)(prev_vao as u32);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, prev_buf as u32);
        (gl.use_program)(prev_prog as u32);
        (gl.active_texture)(GL_TEXTURE0 + BG_UNIT);
        (gl.bind_texture)(GL_TEXTURE_2D, prev_tex as u32);
        (gl.active_texture)(prev_active as u32);
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, prev_read_fb as u32);
        (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, prev_draw_fb as u32);
        (gl.viewport)(prev_vp[0], prev_vp[1], prev_vp[2], prev_vp[3]);
        (gl.blend_equation_separate)(be[0] as u32, be[1] as u32);
        (gl.blend_func_separate)(bs[0] as u32, bs[1] as u32, bs[2] as u32, bs[3] as u32);
        (gl.depth_func)(prev_depth_func as u32);
        (gl.depth_mask)(prev_mask as u8);
        (gl.color_mask)(prev_cmask[0], prev_cmask[1], prev_cmask[2], prev_cmask[3]);
        for (k, &cap) in caps.iter().enumerate() {
            if was[k] != 0 {
                (gl.enable)(cap)
            } else {
                (gl.disable)(cap)
            }
        }
        gl_clear_errors_v(gl);
    }
}
static IN_SCENE_LOGGED: AtomicBool = AtomicBool::new(false);

/// Are the renderer's own OpenGL objects still there (same context, program alive)?
/// Changing resolution / supersampling makes the game rebuild its graphics.
unsafe fn renderer_healthy(r: &Renderer) -> bool {
    unsafe {
        let ctx = (r.gl.get_current_context)() as usize;
        // (textures aren't checked: glIsTexture is false for a texture that was
        //  created but not used yet, which ours legitimately are at first)
        // (the context check is free; asking the driver whether our program still exists is
        // a query - only now and then: it can only go away with the context)
        ctx == RENDERER_CTX.load(Ordering::Relaxed) && (FRAME.load(Ordering::Relaxed) % 256 != 0 || (r.gl.is_program)(r.program) != 0)
    }
}

static LAST_RESET: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Throw the renderer away (it's rebuilt next frame) and repaint the stain maps.
/// At most once every 2 seconds, so a misfiring check can never spin.
fn reset_renderer(cell: &mut RendererCell) {
    if let Ok(mut t) = LAST_RESET.lock() {
        if let Some(last) = *t {
            if last.elapsed().as_secs_f32() < 2.0 {
                return;
            }
        }
        *t = Some(std::time::Instant::now());
    }
    cell.0 = None;
    cell.1 = false;
    if let Ok(mut si) = SCENE_INFO.lock() {
        *si = None;
    }
    if let Ok(mut c) = CMB.lock() {
        *c = (0, 0, (0, 0), 0);
    }
    if let Ok(mut u) = UI_GL.lock() {
        *u = None;
    }
    if let Ok(mut sn) = SNAP.lock() {
        *sn = None;
    }
    CMB_LOGGED.store(false, Ordering::Relaxed);
    if let Ok(mut l) = SCENE_LOGGED.lock() {
        l.clear();
    }
    IN_SCENE_LOGGED.store(false, Ordering::Relaxed);
    if let Ok(mut m) = MAPS.lock() {
        *m = None;
    }
    MAP_DIRTY.store(true, Ordering::Relaxed);
    IN_SCENE_FAILED.store(false, Ordering::Relaxed);
    info_f!("Cruor: the game rebuilt its graphics (settings change) - blood renderer rebuilt");
}

unsafe fn screen_draw(verts: &[f32], vp: &Mat4, collide_vp: &Mat4, scene_fbo: Option<u32>, scene_vp: Option<[i32; 4]>) {
    let mut cell = match RENDERER.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    let healthy = match cell.0.as_ref() {
        Some(r) => unsafe { renderer_healthy(r) },
        None => true,
    };
    if !healthy {
        reset_renderer(&mut cell);
    }
    if cell.0.is_none() {
        if cell.1 {
            return;
        }
        cell.1 = true;
        cell.0 = unsafe { create_renderer() };
        if let Some(r) = cell.0.as_ref() {
            RENDERER_CTX.store(unsafe { (r.gl.get_current_context)() } as usize, Ordering::Relaxed);
        }
    }
    let r = match cell.0.as_mut() {
        Some(r) => r,
        None => return,
    };
    let inv_vp = match mat_inverse(vp) {
        Some(m) => m,
        None => return,
    };

    unsafe {
        let gl = &r.gl;
        // --- in-game stains: refresh the blood map and keep it bound on its unit ---
        // (moving needs no rebuild: the wrap-around maps refill only the strip that comes
        //  into range; a change of floor height refills just the wall maps)
        marker_pixel_report(gl);
        update_blood_map(gl, r.stain_tex, r.wallx_tex, r.wallz_tex);
        update_obj_atlas(gl, r.atlas_tex);
        {
            let prev_active = 0x84C0 + CUR_UNIT.load(Ordering::Relaxed);
            for (unit, tex) in [(stain_unit(), r.stain_tex), (wallx_unit(), r.wallx_tex), (wallz_unit(), r.wallz_tex), (atlas_unit(), r.atlas_tex)] {
                (gl.active_texture)(GL_TEXTURE0 + unit);
                (gl.bind_texture)(GL_TEXTURE_2D, tex);
            }
            (gl.active_texture)(prev_active);
        }
        {
            let mut rect = [0i32; 4];
            let hwnd = WindowFromDC((gl.get_current_dc)());
            if !hwnd.is_null() && GetClientRect(hwnd, &mut rect) != 0 {
                ui_draw(gl, rect[2] - rect[0], rect[3] - rect[1]);
            }
        }
        if verts.is_empty() || !BLOOD_ON.load(Ordering::Relaxed) {
            return;
        }
        // --- save the game's state ---
        let mut prev_prog = 0;
        let mut prev_vao = 0;
        let mut prev_buf = 0;
        let mut prev_draw_fb = 0;
        let mut prev_read_fb = 0;
        let mut prev_vp = [0i32; 4];
        let mut prev_mask = 0;
        let mut prev_depth_func = 0;
        let mut prev_draw_buf = 0;
        let mut prev_read_buf = 0;
        let mut prev_active = 0;
        let mut prev_tex = 0;
        let mut bs = [0i32; 4];
        let mut prev_cmask = [1u8; 4];
        (gl.get_integerv)(GL_CURRENT_PROGRAM, &mut prev_prog);
        (gl.get_integerv)(GL_VERTEX_ARRAY_BINDING, &mut prev_vao);
        (gl.get_integerv)(GL_ARRAY_BUFFER_BINDING, &mut prev_buf);
        (gl.get_integerv)(GL_DRAW_FRAMEBUFFER_BINDING, &mut prev_draw_fb);
        (gl.get_integerv)(GL_READ_FRAMEBUFFER_BINDING, &mut prev_read_fb);
        (gl.get_integerv)(GL_VIEWPORT, prev_vp.as_mut_ptr());
        (gl.get_integerv)(GL_DEPTH_WRITEMASK, &mut prev_mask);
        (gl.get_integerv)(GL_DEPTH_FUNC, &mut prev_depth_func);
        (gl.get_integerv)(GL_DRAW_BUFFER, &mut prev_draw_buf);
        (gl.get_integerv)(GL_READ_BUFFER, &mut prev_read_buf);
        (gl.get_integerv)(GL_ACTIVE_TEXTURE, &mut prev_active);
        (gl.active_texture)(GL_TEXTURE0 + BG_UNIT);
        (gl.get_integerv)(GL_TEXTURE_BINDING_2D, &mut prev_tex);
        (gl.get_integerv)(GL_BLEND_SRC_RGB, &mut bs[0]);
        (gl.get_integerv)(GL_BLEND_DST_RGB, &mut bs[1]);
        (gl.get_integerv)(GL_BLEND_SRC_ALPHA, &mut bs[2]);
        (gl.get_integerv)(GL_BLEND_DST_ALPHA, &mut bs[3]);
        (gl.get_booleanv)(GL_COLOR_WRITEMASK, prev_cmask.as_mut_ptr());
        let was_blend = (gl.is_enabled)(GL_BLEND);
        let was_depth = (gl.is_enabled)(GL_DEPTH_TEST);
        let was_cull = (gl.is_enabled)(GL_CULL_FACE);
        let was_scissor = (gl.is_enabled)(GL_SCISSOR_TEST);
        let was_stencil = (gl.is_enabled)(GL_STENCIL_TEST);
        for _ in 0..16 {
            if (gl.get_error)() == 0 {
                break;
            }
        }

        // --- the window ---
        (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
        (gl.draw_buffer)(GL_BACK);
        let mut rect = [0i32; 4];
        let hwnd = WindowFromDC((gl.get_current_dc)());
        let (w, h) = if !hwnd.is_null() && GetClientRect(hwnd, &mut rect) != 0 {
            (rect[2] - rect[0], rect[3] - rect[1])
        } else {
            (prev_vp[2], prev_vp[3])
        };
        (gl.disable)(GL_SCISSOR_TEST);
        (gl.disable)(GL_STENCIL_TEST);
        (gl.disable)(GL_CULL_FACE);
        (gl.color_mask)(1, 1, 1, 1);

        // --- copy the game's scene depth onto the window ---
        // The game renders the NEXT frame into its scene buffer while this one is on
        // screen, so its depth is one frame ahead of the picture (and of the camera the
        // flying drops are drawn with). Keep each frame's depth and use last frame's.
        let mut use_depth = false;
        if DEPTH_ON.load(Ordering::Relaxed) && DEPTH_COPY_STATE.load(Ordering::Relaxed) != 2 && w > 0 && h > 0 {
            if let (Some(fbo), Some(sv)) = (scene_fbo, scene_vp) {
                (gl.depth_mask)(1);
                let (sw, sh) = (sv[2], sv[3]);
                // (after a failure, use the plain copy for a couple of seconds, then retry)
                if r.dfail && r.dretry > 0 {
                    r.dretry -= 1;
                } else if r.dfail {
                    r.dfail = false;
                    r.dsize = (0, 0);
                    r.dfbo = 0;
                    r.dvalid = false;
                }
                if !r.dfail && (r.dstore[0] == 0 || r.dsize != (sw, sh) || r.dfbo != fbo) {
                    // Make two depth stores in the same format as the game's scene depth.
                    let mut prev_rb = 0;
                    (gl.get_integerv)(0x8CA7, &mut prev_rb); // RENDERBUFFER_BINDING
                    (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, fbo);
                    let mut fmt = 0x88F0; // DEPTH24_STENCIL8
                    for attach in [0x821Au32, 0x8D00] {
                        let mut ty = 0;
                        (gl.get_framebuffer_attachment_parameteriv)(GL_READ_FRAMEBUFFER, attach, 0x8CD0, &mut ty);
                        if ty == 0x8D41 {
                            let mut name = 0;
                            (gl.get_framebuffer_attachment_parameteriv)(GL_READ_FRAMEBUFFER, attach, 0x8CD1, &mut name);
                            (gl.bind_renderbuffer)(0x8D41, name as u32);
                            (gl.get_renderbuffer_parameteriv)(0x8D41, 0x8D44, &mut fmt);
                            break;
                        }
                    }
                    let _ = (gl.get_error)();
                    let attach = if fmt == 0x88F0 || fmt == 0x8CAD { 0x821A } else { 0x8D00 };
                    if r.dstore[0] == 0 {
                        (gl.gen_framebuffers)(2, r.dstore.as_mut_ptr());
                        (gl.gen_renderbuffers)(2, r.drb.as_mut_ptr());
                    }
                    let mut ok = true;
                    for k in 0..2 {
                        (gl.bind_renderbuffer)(0x8D41, r.drb[k]);
                        (gl.renderbuffer_storage)(0x8D41, fmt as u32, sw, sh);
                        (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, r.dstore[k]);
                        (gl.framebuffer_renderbuffer)(GL_DRAW_FRAMEBUFFER, attach, 0x8D41, r.drb[k]);
                        (gl.draw_buffer)(0);
                        (gl.read_buffer)(0);
                        if (gl.check_framebuffer_status)(GL_DRAW_FRAMEBUFFER) != 0x8CD5 {
                            ok = false;
                        }
                    }
                    (gl.bind_renderbuffer)(0x8D41, prev_rb as u32);
                    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
                    (gl.draw_buffer)(GL_BACK);
                    r.dsize = (sw, sh);
                    r.dfbo = fbo;
                    r.dvalid = false;
                    if !ok || (gl.get_error)() != 0 {
                        r.dfail = true;
                        r.dretry = 120;
                        log::debug!("Cruor: depth stores not usable for this view yet; plain copy for now");
                    }
                }
                if !r.dfail {
                    // 1) last frame's depth -> the window (matches the picture on screen)
                    if r.dvalid {
                        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, r.dstore[r.dcur ^ 1]);
                        (gl.blit_framebuffer)(0, 0, sw, sh, 0, 0, w, h, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
                    }
                    // 2) this frame's depth -> keep for next frame
                    (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, fbo);
                    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, r.dstore[r.dcur]);
                    (gl.blit_framebuffer)(sv[0], sv[1], sv[0] + sw, sv[1] + sh, 0, 0, sw, sh, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
                    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
                    (gl.draw_buffer)(GL_BACK);
                    let err = (gl.get_error)();
                    if err == 0 {
                        use_depth = r.dvalid;
                        r.dvalid = true;
                        r.dcur ^= 1;
                        if DEPTH_COPY_STATE.swap(1, Ordering::Relaxed) != 1 {
                            info_f!("Cruor: depth copy works (matched to the picture on screen)");
                        }
                    } else {
                        r.dfail = true;
                        r.dretry = 120;
                        log::debug!("Cruor: stored depth copy failed (OpenGL error 0x{:X}); plain copy for now", err);
                    }
                } else {
                    (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, fbo);
                    (gl.blit_framebuffer)(sv[0], sv[1], sv[0] + sw, sv[1] + sh, 0, 0, w, h, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
                    let err = (gl.get_error)();
                    if err == 0 {
                        use_depth = true;
                        DEPTH_COPY_STATE.store(1, Ordering::Relaxed);
                    } else {
                        DEPTH_COPY_STATE.store(2, Ordering::Relaxed);
                        warn_f!("Cruor: depth copy failed (OpenGL error 0x{:X}); blood will draw on top", err);
                    }
                }
            }
        }
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, 0);
        (gl.read_buffer)(GL_BACK);
        let _ = collide_vp; // (landing is done with the game's own collision now)

        // --- copy the picture so the liquid can refract/tint it ---
        (gl.bind_texture)(GL_TEXTURE_2D, r.bg_tex);
        if r.bg_size != (w, h) {
            (gl.tex_image_2d)(GL_TEXTURE_2D, 0, 0x881A /* RGBA16F */, w, h, 0, GL_RGBA, GL_FLOAT, std::ptr::null());
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            r.bg_size = (w, h);
        }
        (gl.copy_tex_sub_image_2d)(GL_TEXTURE_2D, 0, 0, 0, 0, 0, w, h);

        // --- draw the drops ---
        (gl.viewport)(0, 0, w, h);
        (gl.enable)(GL_BLEND);
        (gl.blend_func_separate)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ZERO, GL_ONE);
        if use_depth {
            (gl.enable)(GL_DEPTH_TEST);
            (gl.depth_func)(GL_LEQUAL);
            (gl.depth_mask)(1);
        } else {
            (gl.disable)(GL_DEPTH_TEST);
            (gl.depth_mask)(0);
        }
        let right = norm3([vp[0], vp[4], vp[8]]);
        let up = norm3([vp[1], vp[5], vp[9]]);
        let fwd = norm3([vp[3], vp[7], vp[11]]);
        let e = mat_mul_vec(&inv_vp, [0.0, 0.0, 1.0, 0.0]);
        let eye = if e[3].abs() > 1e-6 { [e[0] / e[3], e[1] / e[3], e[2] / e[3]] } else { [0.0; 3] };

        (gl.use_program)(r.program);
        (gl.uniform_matrix4fv)(r.u_view_proj, 1, 0, vp.as_ptr());
        (gl.uniform3f)(r.u_right, right[0], right[1], right[2]);
        (gl.uniform3f)(r.u_up, up[0], up[1], up[2]);
        (gl.uniform3f)(r.u_fwd, fwd[0], fwd[1], fwd[2]);
        (gl.uniform3f)(r.u_eye, eye[0], eye[1], eye[2]);
        (gl.uniform2f)(r.u_viewport, w as f32, h as f32);
        (gl.uniform2f)(r.u_vp_origin, 0.0, 0.0);
        (gl.uniform1f)(r.u_have_bg, 1.0);
        (gl.uniform1f)(r.u_scene_space, 0.0);
        (gl.uniform1f)(r.u_drop, 1.0);
        (gl.uniform1f)(r.u_decode_bg, 0.0);
        (gl.uniform1f)(r.u_scale, VISUAL_SCALE);
        (gl.uniform1f)(r.u_stretch, STRETCH);
        (gl.uniform1i)(r.u_bg, BG_UNIT as i32);
        (gl.bind_vertex_array)(r.vao);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, r.vbo);
        (gl.buffer_data)(GL_ARRAY_BUFFER, (verts.len() * 4) as isize, verts.as_ptr() as *const c_void, GL_STREAM_DRAW);
        (gl.draw_arrays)(GL_TRIANGLES, 0, (verts.len() / 9) as i32);
        let err = (gl.get_error)();
        if err != 0 && GL_ERRORS_LOGGED.fetch_add(1, Ordering::Relaxed) < 5 {
            error_f!("Cruor: OpenGL error 0x{:X} while drawing blood", err);
        }

        // --- restore ---
        (gl.bind_vertex_array)(prev_vao as u32);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, prev_buf as u32);
        (gl.use_program)(prev_prog as u32);
        (gl.bind_texture)(GL_TEXTURE_2D, prev_tex as u32);
        (gl.active_texture)(prev_active as u32);
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, prev_read_fb as u32);
        (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, prev_draw_fb as u32);
        (gl.draw_buffer)(prev_draw_buf as u32);
        (gl.read_buffer)(prev_read_buf as u32);
        (gl.viewport)(prev_vp[0], prev_vp[1], prev_vp[2], prev_vp[3]);
        (gl.blend_func_separate)(bs[0] as u32, bs[1] as u32, bs[2] as u32, bs[3] as u32);
        (gl.depth_func)(prev_depth_func as u32);
        (gl.depth_mask)(prev_mask as u8);
        (gl.color_mask)(prev_cmask[0], prev_cmask[1], prev_cmask[2], prev_cmask[3]);
        for (cap, on) in [
            (GL_BLEND, was_blend),
            (GL_DEPTH_TEST, was_depth),
            (GL_CULL_FACE, was_cull),
            (GL_SCISSOR_TEST, was_scissor),
            (GL_STENCIL_TEST, was_stencil),
        ] {
            if on != 0 {
                (gl.enable)(cap)
            } else {
                (gl.disable)(cap)
            }
        }
        for _ in 0..16 {
            if (gl.get_error)() == 0 {
                break;
            }
        }
    }
}


// ============================================================
// IN-GAME STAINS
// Puddles and splats are no longer drawn on top of the picture. Instead, the game's
// own surface shaders are given a "blood map" (a texture covering the floor around
// the action) and darken/redden any upward-facing surface where blood has landed.
// So stains are part of the game's rendering: lit by its lights, under characters'
// feet, behind walls, blurred with the scene, never over the interface.
// ============================================================
const MAP_N: usize = 4096; // 1.17 cm texels (75/64 cm)
const MAP_SIZE: f32 = 4800.0; // cm covered by the map (48 m)
// Texture slots for our three maps: the TOP three slots the graphics card has
// (worked out when the renderer starts), which the game doesn't use.
static UNIT_BASE: AtomicU32 = AtomicU32::new(13);
fn stain_unit() -> u32 { UNIT_BASE.load(Ordering::Relaxed) + 2 }
fn wallx_unit() -> u32 { UNIT_BASE.load(Ordering::Relaxed) + 1 }
fn wallz_unit() -> u32 { UNIT_BASE.load(Ordering::Relaxed) }
fn atlas_unit() -> u32 { UNIT_BASE.load(Ordering::Relaxed) + 3 }
/// Highest texture slot the game itself has assigned to one of its shaders.
static GAME_MAX_UNIT: AtomicU32 = AtomicU32::new(0);
/// A texture slot of OURS the game has assigned to one of its textures (0 = none).
static GAME_USES_OUR_SLOT: AtomicU32 = AtomicU32::new(0);
static UNIT_CONFLICT: AtomicBool = AtomicBool::new(false);
/// Debug view (F11) and stain rules (F12: 0 exclude moving, 1 everything, 2 level only).
static STAIN_DEBUG: AtomicBool = AtomicBool::new(false);
static STAIN_MODE: AtomicU32 = AtomicU32::new(0);
const WALL_N: usize = 6144; // wall map width (texels across MAP_SIZE) - 0.8 cm texels
const WALL_H: usize = 1024; // wall map height (texels) - 0.8 cm texels
const WALL_HEIGHT: f32 = 800.0; // cm of height covered by the wall maps
static WALL_Y: Mutex<[f32; 2]> = Mutex::new([0.0, 1.0 / WALL_HEIGHT]);
static WALL_Y0: Mutex<Option<f32>> = Mutex::new(None);
static LAST_MAP_BUILD: Mutex<Option<std::time::Instant>> = Mutex::new(None);
const GL_RG16F: i32 = 0x822F;
const GL_RG: u32 = 0x8227;

static MAP_DIRTY: AtomicBool = AtomicBool::new(true);
static MAP_RECT: Mutex<[f32; 4]> = Mutex::new([0.0, 0.0, 1.0 / MAP_SIZE, 1.0 / MAP_SIZE]);
static INJECTED_SHADERS: AtomicU32 = AtomicU32::new(0);
static INJECT_FAILURES: AtomicU32 = AtomicU32::new(0);

/// Code added to every main surface shader. The game's own main() is renamed and
/// called first; then the lit colour is stained where the blood map says so.
const STAIN_GLSL: &str = "
uniform sampler2D PnBloodMap;
uniform vec4 PnBloodRect;
uniform float PnBloodOn;
uniform sampler2D PnWallX;
uniform sampler2D PnWallZ;
uniform vec4 PnWallY;
uniform float PnDebug;
uniform float PnDarkness;
uniform sampler2D PnObjAtlas;
uniform vec4 PnObjBox;
uniform vec4 PnObjTile;
uniform int PnObjN;
uniform mat4 PnObjInv;
void main() {
    pn_blood_game_main();
    // (surface direction worked out up front: derivatives need all pixels in step)
    vec3 n = normalize(cross(dFdx(vrPos.xyz), dFdy(vrPos.xyz)));
    float dk = PnDarkness > 0.0 ? PnDarkness : 1.0;
    vec3 pnWet = vec3(0.40, 0.03, 0.025) * (2.0 - clamp(dk, 0.2, 1.8));
    // Blood stuck to this object (movable things and characters), in its own coordinates.
    // Its own blood map: three tiles (one per side direction, in the object's coordinates).
    if (PnObjN > 0) {
        vec3 lp = (PnObjInv * vec4(vrPos.xyz, 1.0)).xyz;
        vec3 q = (lp - PnObjBox.xyz) * PnObjBox.w;
        vec3 ln = abs(mat3(PnObjInv) * n);
        vec2 t;
        float ax;
        if (ln.x >= ln.y && ln.x >= ln.z) { t = q.yz; ax = 0.0; }
        else if (ln.y >= ln.z) { t = q.xz; ax = 1.0; }
        else { t = q.xy; ax = 2.0; }
        if (abs(t.x) < 1.0 && abs(t.y) < 1.0) {
            vec2 uv = PnObjTile.xy + vec2((ax + t.x * 0.5 + 0.5) * PnObjTile.z, (t.y * 0.5 + 0.5) * PnObjTile.w);
            float om = texture(PnObjAtlas, uv).r;
            if (om > 0.001) FragColor.rgb *= mix(vec3(1.0), pnWet, clamp(om, 0.0, 1.0));
        }
    }
    if (PnBloodOn > 0.5 && abs(n.y) < 0.6) {
        // Wall splats: looked up in the wall maps (one for walls facing along z, one along x).
        float wm = 0.0;
        float vy = (vrPos.y - PnWallY.x) * PnWallY.y;
        if (vy > 0.0 && vy < 1.0) {
            if (abs(n.z) >= abs(n.x)) {
                float ux = (vrPos.x - PnBloodRect.x) * PnBloodRect.z;
                if (ux > 0.0 && ux < 1.0) {
                    vec2 w = texture(PnWallX, vec2(vrPos.x * PnBloodRect.z, vy)).rg;
                    wm = w.r * (1.0 - smoothstep(4.0, 10.0, abs(vrPos.z - w.g)));
                }
            } else {
                float uz = (vrPos.z - PnBloodRect.y) * PnBloodRect.w;
                if (uz > 0.0 && uz < 1.0) {
                    vec2 w = texture(PnWallZ, vec2(vrPos.z * PnBloodRect.w, vy)).rg;
                    wm = w.r * (1.0 - smoothstep(4.0, 10.0, abs(vrPos.x - w.g)));
                }
            }
        }
        if (wm > 0.001) FragColor.rgb *= mix(vec3(1.0), pnWet, wm);
    }
    if (PnBloodOn > 0.5) {
        vec2 uv = (vrPos.xz - PnBloodRect.xy) * PnBloodRect.zw;
        if (uv.x > 0.0 && uv.y > 0.0 && uv.x < 1.0 && uv.y < 1.0) {
            vec3 b = texture(PnBloodMap, vrPos.xz * PnBloodRect.zw).rgb;
            if (b.r > 0.002) {
                float up = smoothstep(0.55, 0.8, abs(n.y));
                // Only paint surfaces within a few cm of the stain height (floors, not bodies above them).
                float tol = max(b.b, 8.0);
                float near = 1.0 - smoothstep(tol * 0.4, tol, abs(vrPos.y - b.g));
                float m = clamp(b.r, 0.0, 1.0) * up * near;
                vec3 wet = pnWet;
                FragColor.rgb *= mix(vec3(1.0), wet, m);
            }
        }
    }
}
";

/// Should this shader get the blood-map code? (The game's main surface shaders.)
fn wants_stains(src: &str) -> bool {
    src.contains("VXGIPos")
        && src.contains("out vec4 FragColor")
        && src.contains("in vec4 vrPos")
        && src.matches("void main()").count() == 1
        && !src.contains("PnBloodMap")
}

static REAL_SHADER_SOURCE: AtomicUsize = AtomicUsize::new(0);

// ---- Per-frame hook without touching game code ----
// The game keeps the address of wglSwapBuffers in a fixed slot (image offset of
// 0x100496900) and calls it through that slot at the end of every frame. We put our
// own function in that slot: it does the blood's per-frame work, then calls the real one.
/// Found at startup by reading it out of the game's present function.
static SWAP_SLOT: AtomicUsize = AtomicUsize::new(0);
const SWAP_SLOT_SIG: &str = "48 8D 64 24 D8 48 83 3D ?? ?? ?? ?? 00 74 0F 48 8B 0D ?? ?? ?? ?? FF 15 ?? ?? ?? ?? EB 0D 48 8B 0D ?? ?? ?? ?? FF 15";
static SWAP_REFUSED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
struct MemInfo {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    _pad1: u32,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
    _pad2: u32,
}
#[link(name = "kernel32")]
extern "system" {
    fn VirtualQuery(addr: *const c_void, info: *mut MemInfo, len: usize) -> usize;
    fn GetCurrentThreadId() -> u32;
}

/// The thread the plugin does its per-frame work on (the first one to present), and a
/// guard so that work never runs twice at once.
static FRAME_THREAD: AtomicU32 = AtomicU32::new(0);
static IN_FRAME: AtomicBool = AtomicBool::new(false);
static OTHER_THREAD_PRESENTS: AtomicU32 = AtomicU32::new(0);
/// Does this address point at code (executable memory)?
fn is_code(addr: usize) -> bool {
    let mut mi: MemInfo = unsafe { std::mem::zeroed() };
    let n = unsafe { VirtualQuery(addr as *const c_void, &mut mi, std::mem::size_of::<MemInfo>()) };
    n != 0 && mi.state == 0x1000 && (mi.protect & 0xF0) != 0
}

/// Read a rip-relative address out of the game's code: `disp_at` holds a 32-bit
/// displacement that is relative to `next_ip`.
unsafe fn rip_target(disp_at: usize, next_ip: usize) -> usize {
    let disp = unsafe { (disp_at as *const i32).read_unaligned() } as isize;
    (next_ip as isize + disp) as usize
}
static REAL_SWAP: AtomicUsize = AtomicUsize::new(0);
static SWAP_INSTALLED: AtomicBool = AtomicBool::new(false);
type SwapFn = unsafe extern "system" fn(*mut c_void) -> i32;

unsafe extern "system" fn my_swap_buffers(hdc: *mut c_void) -> i32 {
    let tid = unsafe { GetCurrentThreadId() };
    let owner = match FRAME_THREAD.compare_exchange(0, tid, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => tid,
        Err(t) => t,
    };
    if tid != owner {
        // the game presents from another thread too (e.g. while it streams an area in):
        // the blood's per-frame work stays on its own thread, so nothing runs twice at once
        if OTHER_THREAD_PRESENTS.fetch_add(1, Ordering::Relaxed) == 0 {
            warn_f!(
                "Cruor: the game also presents from another thread ({} - the blood's thread is {}); blood work is skipped there",
                tid, owner
            );
        }
    } else if !IN_FRAME.swap(true, Ordering::Acquire) {
        prepare_frame();
        IN_FRAME.store(false, Ordering::Release);
    }
    let real: SwapFn = unsafe { std::mem::transmute(REAL_SWAP.load(Ordering::Relaxed)) };
    unsafe { real(hdc) }
}

fn install_swap_hook() {
    if SWAP_INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    let slot_addr = SWAP_SLOT.load(Ordering::Relaxed);
    if slot_addr == 0 || SWAP_REFUSED.load(Ordering::Relaxed) {
        return;
    }
    let slot = slot_addr as *mut usize;
    let current = unsafe { slot.read_unaligned() };
    if current == 0 || current == my_swap_buffers as *const () as usize {
        return; // not set up by the game yet
    }
    // Sanity check: it should point at opengl32's wglSwapBuffers.
    let expected = unsafe {
        let ogl = GetModuleHandleA(b"opengl32.dll\0".as_ptr() as *const c_char);
        if ogl.is_null() { 0 } else { GetProcAddress(ogl, b"wglSwapBuffers\0".as_ptr() as *const c_char) as usize }
    };
    if expected != 0 && current != expected {
        if is_code(current) {
            // Something else (an overlay tool) already wraps it - chain onto theirs.
            info_f!("Cruor: frame function already wrapped by another tool; chaining after it");
        } else {
            // Not what we expect at all: don't touch it.
            warn_f!("Cruor: this Exanima version doesn't match what the mod expects - blood is switched off (the game is unaffected)");
            SWAP_REFUSED.store(true, Ordering::Relaxed);
            return;
        }
    }
    REAL_SWAP.store(current, Ordering::Relaxed);
    unsafe { slot.write_unaligned(my_swap_buffers as *const () as usize) };
    SWAP_INSTALLED.store(true, Ordering::Relaxed);
    info_f!("Cruor: per-frame hook installed (no game code patched)");
}
static REAL_UNIFORM1I: AtomicUsize = AtomicUsize::new(0);
type Uniform1iFn = unsafe extern "system" fn(i32, i32);

/// The game's own glUniform1i: this is how it assigns texture slots to its shaders.
/// If it ever uses one of ours, stains are switched off rather than break the game.
// ---- Blood on movable objects and characters ----
// Every movable object the game draws is followed from frame to frame by its position.
// Splats that hit one are stored in that object's own coordinates, and handed to the
// game's shader whenever it draws that object, so the blood moves with it.
struct Track {
    mat: [f32; 16],
    key: Option<(u32, i32, usize)>, // which 3D model it is: (vertex array, index count, index offset)
    vel: [f32; 3], // movement per frame, for predicting where it is next
    seen: u32,
    splats: Vec<[f32; 4]>, // local position + local radius
}
static TRACKS: Mutex<Vec<Track>> = Mutex::new(Vec::new());
static FRAME_OBJECTS: Mutex<Vec<([f32; 16], bool)>> = Mutex::new(Vec::new());
static LAST_OBJECTS: Mutex<Vec<([f32; 16], bool)>> = Mutex::new(Vec::new());
static OBJ_LOCS: Mutex<Option<std::collections::HashMap<u32, (i32, i32, i32, i32)>>> = Mutex::new(None);
static OBJECT_HITS: AtomicU32 = AtomicU32::new(0);


// ============================================================
// GAME COLLISION
// Every level model (TMesh / TTerrainMesh) carries the game's own collision functions
// as (function, object) pairs; slot +0x3B0 is a ray test - the fast kd-tree version
// when the model has a TkdTree (installed at 0x100268CE0), otherwise the plain one
// (0x1000605A0). Ray record: max distance +0x0C, start +0x10, direction +0x20, all in
// the model's own coordinates; returns the triangle hit (or -1) and writes the hit
// distance back into +0x0C.
// Placing a model: world = object offset (object +0x64) + model offset (model +0x64)
// + rotated local point (rotation rows at model +0x40, +0x4C, +0x58).
// Bounding sphere: centre at model +0x20 (object coordinates), radius at +0x2C.
// Loose props have a rigid body (model +0x160 != 0) and move; level pieces don't.
// ============================================================
const COL_CELL: f32 = 100.0;
/// At most this many collision tests per frame; the rest wait a frame (never stalls).
const RAY_BUDGET: usize = 400;
#[allow(dead_code)]
type RayFn = unsafe extern "C" fn(*mut c_void, *mut f32) -> i32;

#[derive(Clone, Copy)]
struct PartRef {
    part: usize,
    obj: usize,
    vmt: usize,
    center: [f32; 3], // world (static pieces only)
    radius: f32,
}
struct LevelIndex {
    arr: usize,
    hi: i32,
    stat: Vec<PartRef>,
    stamp: Vec<u32>, // per static piece: last query that checked it (fast "already checked")
    query: u32,
    cells: std::collections::HashMap<(i32, i32), Vec<u32>>,
    big: Vec<u32>,
    props: Vec<PartRef>,
    // props move, so their grid is rebuilt every few frames from where they are now
    pcells: std::collections::HashMap<(i32, i32), Vec<u32>>,
    pbig: Vec<u32>,
    ppos: Vec<([f32; 3], f32)>, // current world centre, radius (0 = gone)
    pstamp: Vec<u32>,
    pframe: u32,
    /// Pieces not used for collision: (piece, object, reason). For the stairs probe.
    skipped: Vec<(usize, usize, u32)>,
    /// Where every fixed level piece is placed (world position, 1 cm grid): lets a draw
    /// of a level piece built at an angle (e.g. rotated stairs) still get floor stains.
    static_keys: std::collections::HashSet<(i32, i32, i32)>,
}
static LEVEL: Mutex<Option<LevelIndex>> = Mutex::new(None);
static COL_FLOOR: AtomicU32 = AtomicU32::new(0);
static COL_WALL: AtomicU32 = AtomicU32::new(0);
static COL_PROP: AtomicU32 = AtomicU32::new(0);
static COL_BODY: AtomicU32 = AtomicU32::new(0);
/// At most this long (s) of drop collision per frame; drops that miss their turn have
/// their whole path tested next frame, so nothing slips through.
const COLLISION_BUDGET_S: f32 = 0.002;

// ============================================================
// BLOOD ON CHARACTERS (all from the exe)
// - the game's add-blood-entry 0x100157D90 (character, &world position, anchor part,
//   amount): queues an entry (max 7, +0x89C) that its hit processor applies 7 frames
//   later to the character's body (0x1000D2470) and worn items (0x100062DC0). Found by
//   the bytes after its first 15 (the hook overwrites the start).
// - characters: the game's sector list [player +0x5F0] +0x68 (TMotileDynamics entries).
// - body parts: character +0x18 = model root mesh (0x1000E6A00 sets root +0x160 = the
//   character); part builder 0x1000E5230 puts each child mesh's TMotilePart at +0x160.
// - a part is a tetrahedron: 4 world corners at +0xB0 (read by the game's 0x10001CA40).
// ============================================================
const ADD_BLOOD_TAIL: &str = "0F 28 F3 48 8B 02 48 89 44 24 20 8B 42 08 89 44 24 28 83 BB ?? ?? 00 00 07";
static ADD_BLOOD_ADDR: AtomicUsize = AtomicUsize::new(0);
static ADD_BLOOD_LOOKED: AtomicBool = AtomicBool::new(false);
type AddBloodFn = unsafe extern "C" fn(*mut c_void, *const f32, *mut c_void, f32);
/// Drops ignore bodies for this long after leaving the wound (they start inside it).
const BODY_GRACE: f32 = 0.12;

#[derive(Clone, Copy)]
struct BodyPart {
    ch: usize,
    part: usize,
    v: [[f32; 3]; 4],
    centre: [f32; 3],
    radius: f32,
}

fn add_blood_fn() -> Option<AddBloodFn> {
    if !ADD_BLOOD_LOOKED.swap(true, Ordering::Relaxed) {
        match unsafe { find_pattern(ADD_BLOOD_TAIL) } {
            Some(a) => {
                ADD_BLOOD_ADDR.store(a - 15, Ordering::Relaxed);
                if !RELEASE {
                    info_f!("Cruor: found the game's add-blood-entry at 0x{:X} - drops can bloody characters", a - 15);
                }
            }
            None => warn_f!("Cruor: the game's add-blood-entry wasn't found - drops won't bloody characters"),
        }
    }
    let a = ADD_BLOOD_ADDR.load(Ordering::Relaxed);
    if a == 0 { None } else { Some(unsafe { std::mem::transmute::<usize, AddBloodFn>(a) }) }
}

/// Which characters and parts exist is rebuilt every BODY_REBUILD frames (walks the
/// game's sector list and each character's mesh tree); in between only each part's 4
/// corners are re-read. Class checks compare class-table pointers taken from live objects
/// (the player is a TMotileDynamics; its parts are TMotilePart).
const BODY_REBUILD: u32 = 60;
struct BodyCache {
    frame: u32,
    list: Vec<(usize, usize)>, // (character, part)
    vmt_char: usize,
    vmt_part: usize,
    /// The sector-list scan, a slice per frame: next entry, the list, its capacity, and
    /// the characters found so far this pass.
    scan_i: usize,
    scan_arr: usize,
    scan_cap: usize,
    scan_found: Vec<usize>,
    scanning: bool,
}
static BODY_CACHE: Mutex<BodyCache> = Mutex::new(BodyCache {
    frame: 0,
    list: Vec::new(),
    vmt_char: 0,
    vmt_part: 0,
    scan_i: 0,
    scan_arr: 0,
    scan_cap: 0,
    scan_found: Vec::new(),
    scanning: false,
});
/// Sector-list entries looked at per frame.
const BODY_SCAN_SLICE: usize = 32;

/// Start a new pass over the player's sector list.
fn body_scan_begin(c: &mut BodyCache) {
    c.scanning = false;
    c.scan_found.clear();
    c.scan_i = 0;
    if !LAYOUT_OK.load(Ordering::Relaxed) {
        return;
    }
    let slot = PLAYER_SLOT.load(Ordering::Relaxed);
    if slot == 0 {
        return;
    }
    unsafe {
        let mut player = (slot as *const usize).read_unaligned();
        if player == 0 || !readable(player, 0x600) {
            // no player character (spectating): the last character who bled
            player = LAST_BLEEDER.load(Ordering::Relaxed);
            if player == 0 || !readable(player, 0x600) {
                return;
            }
        }
        c.vmt_char = (player as *const usize).read_unaligned();
        let sector = ((player + sector_off()) as *const usize).read_unaligned();
        if sector == 0 || !readable(sector, 0x78) {
            return;
        }
        let arr = ((sector + 0x68) as *const usize).read_unaligned();
        let cap = (((sector + 0x70) as *const i32).read_unaligned()).clamp(0, 8192) as usize;
        if arr == 0 || cap == 0 || !readable(arr, cap * 8) {
            return;
        }
        c.scan_arr = arr;
        c.scan_cap = cap;
        c.scanning = true;
    }
}
/// Look at the next slice of the list; at the end, rebuild the body-part list.
fn body_scan_step(c: &mut BodyCache) {
    if !c.scanning {
        return;
    }
    unsafe {
        let end = (c.scan_i + BODY_SCAN_SLICE).min(c.scan_cap);
        let mut done = end >= c.scan_cap;
        for i in c.scan_i..end {
            let e = ((c.scan_arr + i * 8) as *const usize).read_unaligned();
            if e < 0x10000 || e % 8 != 0 || e > 0x7FFF_FFFF_0000 {
                done = true;
                break;
            }
            if !readable(e, 0x20) {
                done = true;
                break;
            }
            if (e as *const usize).read_unaligned() == c.vmt_char && !c.scan_found.contains(&e) {
                c.scan_found.push(e);
            }
        }
        c.scan_i = end;
        if !done {
            return;
        }
        // the pass is complete: the found characters' body parts
        c.scanning = false;
        c.list.clear();
        let found = std::mem::take(&mut c.scan_found);
        for &ch in found.iter() {
            let mut parts = Vec::new();
            body_parts_fast(ch, &mut parts);
            for p in parts {
                if p == ch {
                    continue;
                }
                let vmt = (p as *const usize).read_unaligned();
                if c.vmt_part == 0 {
                    if readable(p, 0x10) && obj_class_name(p) == "TMotilePart" {
                        c.vmt_part = vmt;
                    } else {
                        continue;
                    }
                }
                if vmt == c.vmt_part {
                    c.list.push((ch, p));
                }
            }
        }
        BODY_CHARS.store(found.len() as u32, Ordering::Relaxed);
        BODY_PARTS.store(c.list.len() as u32, Ordering::Relaxed);
        c.scan_found = found;
    }
}
static BODY_CHARS: AtomicU32 = AtomicU32::new(0);
static BODY_PARTS: AtomicU32 = AtomicU32::new(0);

#[allow(dead_code)]
fn rebuild_bodies(c: &mut BodyCache) {
    c.list.clear();
    let slot = PLAYER_SLOT.load(Ordering::Relaxed);
    if slot == 0 {
        return;
    }
    unsafe {
        let mut player = (slot as *const usize).read_unaligned();
        if player == 0 || !readable(player, 0x600) {
            // no player character (spectating): the last character who bled
            player = LAST_BLEEDER.load(Ordering::Relaxed);
            if player == 0 || !readable(player, 0x600) {
                return;
            }
        }
        c.vmt_char = (player as *const usize).read_unaligned();
        let sector = ((player + sector_off()) as *const usize).read_unaligned();
        if sector == 0 || !readable(sector, 0x70) {
            return;
        }
        let arr = ((sector + 0x68) as *const usize).read_unaligned();
        // the list's capacity (+0x70) bounds how far it can be read; checked once
        let cap = (((sector + 0x70) as *const i32).read_unaligned()).clamp(0, 4096) as usize;
        if arr == 0 || cap == 0 || !readable(arr, cap * 8) {
            return;
        }
        let mut chars = 0;
        let mut pages: Vec<usize> = Vec::new(); // memory pages already checked this scan
        for i in 0..cap {
            let e = ((arr + i * 8) as *const usize).read_unaligned();
            if e < 0x10000 || e % 8 != 0 || e > 0x7FFF_FFFF_0000 {
                break;
            }
            let page = e & !0xFFF;
            if !pages.contains(&page) {
                if !readable(e, 0x20) {
                    break;
                }
                pages.push(page);
                if (e + 0x20) & !0xFFF != page {
                    pages.push((e + 0x20) & !0xFFF);
                }
            }
            if (e as *const usize).read_unaligned() != c.vmt_char {
                continue;
            }
            chars += 1;
            let mut parts = Vec::new();
            body_parts_fast(e, &mut parts);
            for p in parts {
                if p == e {
                    continue; // (the root mesh's +0x160 is the character itself)
                }
                let vmt = (p as *const usize).read_unaligned();
                if c.vmt_part == 0 {
                    // learn the part class once, by name
                    if readable(p, 0x10) && obj_class_name(p) == "TMotilePart" {
                        c.vmt_part = vmt;
                    } else {
                        continue;
                    }
                }
                if vmt == c.vmt_part {
                    c.list.push((e, p));
                }
            }
        }
        BODY_CHARS.store(chars, Ordering::Relaxed);
        BODY_PARTS.store(c.list.len() as u32, Ordering::Relaxed);
    }
}

/// The same walk as body_parts_of, with plain reads (the game's own live structures,
/// read on the game's thread) and no class-name lookups: collects every +0x160 pointer
/// in the character's body tree; the caller keeps the TMotilePart ones.
unsafe fn body_parts_fast(ch: usize, out: &mut Vec<usize>) {
    fn ok(p: usize) -> bool {
        p > 0x10000 && p % 8 == 0 && p < 0x7FFF_FFFF_0000
    }
    // visited meshes + a hard cap: a stale or corrupted tree (cycles, bogus child
    // lists) can never make this walk explode
    unsafe fn walk(mesh: usize, depth: u32, out: &mut Vec<usize>, seen: &mut Vec<usize>) {
        unsafe {
            if depth > 24 || !ok(mesh) || seen.len() >= 512 || seen.contains(&mesh) {
                return;
            }
            seen.push(mesh);
            let part = ((mesh + 0x160) as *const usize).read_unaligned();
            if ok(part) && !out.contains(&part) {
                out.push(part);
            }
            let hi = ((mesh + 0x118) as *const i32).read_unaligned();
            let arr = ((mesh + 0x120) as *const usize).read_unaligned();
            if !(0..256).contains(&hi) || !ok(arr) {
                return;
            }
            for i in 0..=(hi as usize) {
                let c = ((arr + i * 8) as *const usize).read_unaligned();
                if !ok(c) {
                    continue;
                }
                let ty = ((c + 8) as *const u32).read_unaligned();
                let fl = ((c + 0xC) as *const u32).read_unaligned();
                if ty & 0xFFF == 0x89 && fl & 0x200000 != 0 {
                    walk(c, depth + 1, out, seen);
                }
            }
        }
    }
    unsafe {
        let root = ((ch + 0x18) as *const usize).read_unaligned();
        let mut seen = Vec::with_capacity(64);
        walk(root, 0, out, &mut seen);
    }
}

/// Forget every character and body part (level unloaded: they're freed).
fn forget_bodies() {
    if let Ok(mut c) = BODY_CACHE.lock() {
        c.list.clear();
        c.vmt_part = 0;
        c.scanning = false;
        c.scan_found.clear();
    }
    BODY_CHARS.store(0, Ordering::Relaxed);
    BODY_PARTS.store(0, Ordering::Relaxed);
}

/// The body parts (tetrahedra) of the characters in the player's sector, this frame.
fn collect_bodies() -> Vec<BodyPart> {
    let mut out = Vec::new();
    let Ok(mut c) = BODY_CACHE.lock() else { return out };
    c.frame = c.frame.wrapping_add(1);
    // the list is rescanned continuously, a slice per frame (never all at once)
    let t = std::time::Instant::now();
    if !c.scanning && c.frame % BODY_REBUILD == 0 {
        body_scan_begin(&mut c);
    } else if !c.scanning && c.list.is_empty() && c.frame % BODY_REBUILD == 1 {
        body_scan_begin(&mut c);
    }
    body_scan_step(&mut c);
    cost_note(2, t.elapsed().as_secs_f64() * 1000.0, 0);
    let vp = c.vmt_part;
    for &(ch, p) in c.list.iter() {
        unsafe {
            if (p as *const usize).read_unaligned() != vp {
                continue;
            }
            let c0 = corners();
            let v = [rv3(p + c0), rv3(p + c0 + 0xC), rv3(p + c0 + 0x18), rv3(p + c0 + 0x24)];
            if !v.iter().all(|q| q.iter().all(|x| x.is_finite())) {
                continue;
            }
            let cen = [
                (v[0][0] + v[1][0] + v[2][0] + v[3][0]) * 0.25,
                (v[0][1] + v[1][1] + v[2][1] + v[3][1]) * 0.25,
                (v[0][2] + v[1][2] + v[2][2] + v[3][2]) * 0.25,
            ];
            let r = v.iter().map(|q| { let d = sub3(*q, cen); dot3(d, d).sqrt() }).fold(0.0f32, f32::max);
            out.push(BodyPart { ch, part: p, v, centre: cen, radius: r });
        }
    }
    out
}

/// Re-check a character and part right before handing them to the game.
fn body_still_valid(bp: &BodyPart) -> bool {
    let Ok(c) = BODY_CACHE.lock() else { return false };
    unsafe {
        readable(bp.ch, blood_arr())
            && (bp.ch as *const usize).read_unaligned() == c.vmt_char
            && readable(bp.part, corners() + 0x30)
            && (bp.part as *const usize).read_unaligned() == c.vmt_part
    }
}

/// Where along p0 -> p0+d (0..1) does the segment enter a tetrahedron? (faces tested both
/// sides; a segment ending inside counts at its end)
fn segment_tetra(p0: [f32; 3], d: [f32; 3], v: &[[f32; 3]; 4]) -> Option<f32> {
    let faces = [[0usize, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];
    let mut best: Option<f32> = None;
    for f in faces.iter() {
        let (a, b, c) = (v[f[0]], v[f[1]], v[f[2]]);
        let e1 = sub3(b, a);
        let e2 = sub3(c, a);
        let pv = cross3(d, e2);
        let det = dot3(e1, pv);
        if det.abs() < 1e-9 {
            continue;
        }
        let inv = 1.0 / det;
        let tv = sub3(p0, a);
        let u = dot3(tv, pv) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let qv = cross3(tv, e1);
        let w = dot3(d, qv) * inv;
        if w < 0.0 || u + w > 1.0 {
            continue;
        }
        let t = dot3(e2, qv) * inv;
        if (0.0..=1.0).contains(&t) && best.map(|b| t < b).unwrap_or(true) {
            best = Some(t);
        }
    }
    best
}

static BODY_HITS_LOGGED: AtomicU32 = AtomicU32::new(0);
/// Drop hits waiting to be handed to the game: (character, part, position, amount).
/// They're given to the game's add-blood-entry from INSIDE its own hit processor for
/// that character (the game's thread, at the moment it handles that character), never
/// from the blood's thread - the game's simulation must not be touched concurrently.
static BODY_QUEUE: Mutex<Vec<(usize, usize, [f32; 3], f32)>> = Mutex::new(Vec::new());
fn forget_body_queue() {
    if let Ok(mut q) = BODY_QUEUE.lock() {
        q.clear();
    }
}
/// Called from the game's hit processor hook (game thread) for character `ch`.
fn hand_queued_hits(ch: usize) {
    let mine: Vec<(usize, usize, [f32; 3], f32)> = match BODY_QUEUE.try_lock() {
        Ok(mut q) => {
            if q.is_empty() {
                return;
            }
            let (m, rest): (Vec<_>, Vec<_>) = q.drain(..).partition(|e| e.0 == ch);
            *q = rest;
            m
        }
        Err(_) => return,
    };
    let Some(f) = add_blood_fn() else { return };
    // at most a few per frame (the game keeps 7 pending entries per character)
    for (c, part, at, amount) in mine.into_iter().take(4) {
        let prev = STAGE.swap(17, Ordering::Relaxed);
        unsafe { f(c as *mut c_void, at.as_ptr(), part as *mut c_void, amount) };
        STAGE.store(prev, Ordering::Relaxed);
    }
}
/// A drop hit a character's body part: queue it for the game (see BODY_QUEUE).
fn drop_hit_body(bp: &BodyPart, at: [f32; 3], amount: f32) {
    if !body_still_valid(bp) {
        return;
    }
    if let Ok(mut q) = BODY_QUEUE.lock() {
        if q.len() < 64 {
            q.push((bp.ch, bp.part, at, amount));
        }
    }
    COL_BODY.fetch_add(1, Ordering::Relaxed);
    if !RELEASE && BODY_HITS_LOGGED.fetch_add(1, Ordering::Relaxed) < 5 {
        info_f!(
            "Cruor: drop hit character 0x{:X} (part 0x{:X}) at ({:.1}, {:.1}, {:.1}) - queued for the game, amount {:.3}",
            bp.ch, bp.part, at[0], at[1], at[2], amount
        );
    }
}

unsafe fn rf32(a: usize) -> f32 {
    unsafe { (a as *const f32).read_unaligned() }
}
unsafe fn rv3(a: usize) -> [f32; 3] {
    unsafe { [rf32(a), rf32(a + 4), rf32(a + 8)] }
}
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Current placement of a model: (rotation rows r0, r1, r2, world offset).
unsafe fn part_pose(p: &PartRef) -> ([f32; 3], [f32; 3], [f32; 3], [f32; 3]) {
    unsafe {
        let off = rv3(p.obj + 0x64);
        let r0 = rv3(p.part + 0x40);
        let r1 = rv3(p.part + 0x4C);
        let r2 = rv3(p.part + 0x58);
        let t = rv3(p.part + 0x64);
        (r0, r1, r2, [off[0] + t[0], off[1] + t[1], off[2] + t[2]])
    }
}

/// Is this a model with a usable ray test? Returns its class pointer.
unsafe fn usable_part(part: usize, code_ok: &mut std::collections::HashMap<usize, bool>) -> Option<usize> {
    if part < 0x10000 || part % 8 != 0 || !readable(part, 0x420) {
        return None;
    }
    unsafe {
        let ty = (part as *const i32).add(2).read_unaligned();
        if ty & 0x89 != 0x89 {
            return None;
        }
        let fl = ((part + 0xC) as *const u32).read_unaligned();
        if fl & 0x7000_0000 == 0x5000_0000 {
            return None; // a special kind the game tests differently
        }
        let f = ((part + 0x3B0) as *const usize).read_unaligned();
        let c = ((part + 0x3B8) as *const usize).read_unaligned();
        if f == 0 || c == 0 {
            return None;
        }
        let ok = *code_ok.entry(f).or_insert_with(|| is_code(f));
        if !ok {
            return None;
        }
        Some((part as *const usize).read_unaligned())
    }
}

/// Does this model belong to a loose object? (Its +0x160 points at a rigid body.)
unsafe fn has_rigid_body(part: usize) -> bool {
    unsafe {
        let rbd = ((part + 0x160) as *const usize).read_unaligned();
        rbd != 0 && readable(rbd, 8) && obj_class_name(rbd).contains("RBD")
    }
}

/// Add every usable model of objects [from..=to] to the index.
unsafe fn index_objects(idx: &mut LevelIndex, from: usize, to: usize, code_ok: &mut std::collections::HashMap<usize, bool>) -> usize {
    let mut skipped = 0;
    unsafe {
        for i in from..=to {
            let obj = ((idx.arr + i * 8) as *const usize).read_unaligned();
            if obj == 0 || !readable(obj, 0x1D8) {
                continue;
            }
            let off = rv3(obj + 0x64);
            let phi = ((obj + 0x1C8) as *const i32).read_unaligned();
            let parr = ((obj + 0x1D0) as *const usize).read_unaligned();
            if !(0..100000).contains(&phi) || parr == 0 || !readable(parr, (phi as usize + 1) * 8) {
                continue;
            }
            for j in 0..=(phi as usize) {
                let part = ((parr + j * 8) as *const usize).read_unaligned();
                let vmt = match usable_part(part, code_ok) {
                    Some(v) => v,
                    None => {
                        skipped += 1;
                        if idx.skipped.len() < 20000 && part > 0x10000 && readable(part, 0x40) {
                            let ty = (part as *const i32).add(2).read_unaligned();
                            let fl = ((part + 0xC) as *const u32).read_unaligned();
                            let reason = if ty & 0x89 != 0x89 { 1 } else if fl & 0x7000_0000 == 0x5000_0000 { 2 } else { 3 };
                            idx.skipped.push((part, obj, reason));
                        }
                        continue;
                    }
                };
                let c = rv3(part + 0x20);
                let r = rf32(part + 0x2C);
                if !(r.is_finite() && r > 0.0 && c.iter().all(|v| v.is_finite())) {
                    skipped += 1;
                    continue;
                }
                let pr = PartRef { part, obj, vmt, center: [off[0] + c[0], off[1] + c[1], off[2] + c[2]], radius: r };
                if has_rigid_body(part) {
                    idx.props.push(pr);
                    continue;
                }
                let k = idx.stat.len() as u32;
                idx.stat.push(pr);
                idx.stamp.push(0);
                let (_, _, _, t) = part_pose(&pr);
                if t.iter().all(|v| v.is_finite()) {
                    idx.static_keys.insert((t[0].round() as i32, t[1].round() as i32, t[2].round() as i32));
                }
                if r > 800.0 {
                    idx.big.push(k);
                    continue;
                }
                let (x0, x1) = (((pr.center[0] - r) / COL_CELL).floor() as i32, ((pr.center[0] + r) / COL_CELL).floor() as i32);
                let (z0, z1) = (((pr.center[2] - r) / COL_CELL).floor() as i32, ((pr.center[2] + r) / COL_CELL).floor() as i32);
                for cx in x0..=x1 {
                    for cz in z0..=z1 {
                        idx.cells.entry((cx, cz)).or_default().push(k);
                    }
                }
            }
        }
    }
    skipped
}

/// TEST: log the game's own blood-on-mesh calls (first 40, then a count every 5 s).
static BOM_LOGGED: AtomicU32 = AtomicU32::new(0);
pub(crate) fn note_blood_on_mesh(part: usize, pos: *const f32, dir: *const f32, amount: f32) {
    if RELEASE {
        return;
    }
    let n = BOM_LOGGED.fetch_add(1, Ordering::Relaxed);
    if n >= 40 {
        return;
    }
    unsafe {
        let rd = |p: *const f32| -> [f32; 3] {
            if p.is_null() { [f32::NAN; 3] } else { unsafe { [p.read_unaligned(), p.add(1).read_unaligned(), p.add(2).read_unaligned()] } }
        };
        let (ps, ds) = (rd(pos), rd(dir));
        let (ty, fl) = if readable(part, 0x10) {
            ((part as *const i32).add(2).read_unaligned(), ((part + 0xC) as *const u32).read_unaligned())
        } else {
            (0, 0)
        };
        info_f!(
            "Cruor GAME blood-on-mesh #{}: part 0x{:X} {} type 0x{:X} flags 0x{:08X}; position ({:.1}, {:.1}, {:.1}); direction ({:.3}, {:.3}, {:.3}); amount {:.3}",
            n, part, if readable(part, 8) { obj_class_name(part) } else { "?".into() }, ty, fl,
            ps[0], ps[1], ps[2], ds[0], ds[1], ds[2], amount
        );
    }
}

/// TEST: when the game sprays blood from a hit character, read the hit record the game
/// stores on that character (+0x8A0: bone object, hit point in bone space, strength) and
/// find where the character keeps that bone: directly in a field, or in a list (one or
/// two pointers deep). First 6 hits are reported.
static BONE_PROBES: AtomicU32 = AtomicU32::new(0);
static HITPROC_SEEN: AtomicBool = AtomicBool::new(false);
static LAST_HIT: Mutex<(usize, usize, [u32; 3])> = Mutex::new((0, 0, [0; 3]));

/// Entry of the game's hit processor: when the record holds a fresh hit the game is about
/// to process (bone set, its wait counter at +0x24 not positive), probe it once.
/// Every character, every update (game thread): queued drop hits.
pub(crate) fn on_char_update(ch: usize) {
    hand_queued_hits(ch);
}

pub(crate) fn on_hit_processor(ch: usize) {
    if !RELEASE && !HITPROC_SEEN.swap(true, Ordering::Relaxed) {
        info_f!("Cruor: watching the game's hit processor");
    }
    let _ = (ch, &LAST_HIT);
}

/// The game adds a blood entry to a character: log exactly what it passes, then find
/// where the character keeps that bone. First 6 calls.
pub(crate) fn on_add_blood_entry(ch: usize, pos: *const f32, bone: usize, amount: f32) {
    if RELEASE || BONE_PROBES.load(Ordering::Relaxed) >= 6 {
        return;
    }
    unsafe {
        let p = if pos.is_null() { [f32::NAN; 3] } else { [pos.read_unaligned(), pos.add(1).read_unaligned(), pos.add(2).read_unaligned()] };
        info_f!(
            "Cruor GAME add-blood-entry: character 0x{:X} ({}), position ({:.1}, {:.1}, {:.1}), bone 0x{:X} ({}), amount {:.3}, entries already pending {}",
            ch, if readable(ch, 8) { obj_class_name(ch) } else { "?".into() },
            p[0], p[1], p[2], bone, if bone != 0 && readable(bone, 8) { obj_class_name(bone) } else { "?".into() },
            amount, if readable(ch + blood_n(), 4) { ((ch + blood_n()) as *const i32).read_unaligned() } else { -1 }
        );
    }
    if BONE_PROBES.load(Ordering::Relaxed) == 0 {
        probe_body_tree(ch, bone);
    }
    BONE_PROBES.fetch_add(1, Ordering::Relaxed);
}

/// The game's own character -> body parts path: the hit processor passes [character
/// +0x18] to 0x1000D0BC0, which treats it as the model's root mesh (children: +0x118 hi
/// index, +0x120 array; name text at +0x140). The part builder 0x1000E5230 walks such a
/// tree (children of type & 0xFFF == 0x89 with flag 0x200000, recursing) and puts each
/// node's TMotilePart at +0x160.
unsafe fn body_parts_of(ch: usize, out: &mut Vec<usize>) {
    unsafe fn walk(mesh: usize, depth: u32, out: &mut Vec<usize>, seen: &mut Vec<usize>) {
        unsafe {
            if depth > 24 || mesh == 0 || seen.len() >= 512 || seen.contains(&mesh) || !readable(mesh, 0x168) {
                return;
            }
            seen.push(mesh);
            let part = ((mesh + 0x160) as *const usize).read_unaligned();
            if part != 0 && !out.contains(&part) && readable(part, 8) && obj_class_name(part) == "TMotilePart" {
                out.push(part);
            }
            let hi = ((mesh + 0x118) as *const i32).read_unaligned();
            let arr = ((mesh + 0x120) as *const usize).read_unaligned();
            if !(0..256).contains(&hi) || arr == 0 || !readable(arr, (hi as usize + 1) * 8) {
                return;
            }
            for i in 0..=(hi as usize) {
                let c = ((arr + i * 8) as *const usize).read_unaligned();
                if c == 0 || !readable(c, 0x168) {
                    continue;
                }
                let ty = ((c + 8) as *const u32).read_unaligned();
                let fl = ((c + 0xC) as *const u32).read_unaligned();
                if ty & 0xFFF == 0x89 && fl & 0x200000 != 0 {
                    walk(c, depth + 1, out, seen);
                }
            }
        }
    }
    unsafe {
        if !readable(ch, 0x20) {
            return;
        }
        let root = ((ch + 0x18) as *const usize).read_unaligned();
        let mut seen = Vec::with_capacity(64);
        walk(root, 0, out, &mut seen);
    }
}

/// TEST (first hit only): check the game's character -> body parts path against the
/// sector list and the part the game hit.
fn probe_body_tree(ch: usize, bone: usize) {
    unsafe {
        let root = if readable(ch, 0x20) { ((ch + 0x18) as *const usize).read_unaligned() } else { 0 };
        // exe (0x1000E6A00): the body's root mesh gets mesh +0x160 = its TMotileDynamics.
        // If [character+0x18] is that root mesh, its +0x160 points back to the character.
        let back = if root != 0 && readable(root, 0x168) { ((root + 0x160) as *const usize).read_unaligned() } else { 0 };
        info_f!(
            "Cruor TREE   root mesh +0x160 = 0x{:X} -> {}",
            back, if back == ch { "points back to the character: +0x18 IS the body's root mesh" } else { "does NOT point back to the character" }
        );
        let mut parts = Vec::new();
        body_parts_of(ch, &mut parts);
        let classes: Vec<String> = parts.iter().take(40).map(|p| if readable(*p, 8) { obj_class_name(*p) } else { "?".into() }).collect();
        let n_motile = classes.iter().filter(|c| c.as_str() == "TMotilePart").count();
        info_f!(
            "Cruor TREE character 0x{:X}: model root mesh +0x18 = 0x{:X} ({}); tree gives {} parts ({} TMotilePart); hit part 0x{:X} {}",
            ch, root, if root != 0 && readable(root, 8) { obj_class_name(root) } else { "-".into() },
            parts.len(), n_motile, bone, if parts.contains(&bone) { "IS among them" } else { "is NOT among them" }
        );
        // compare with the sector list: which characters' parts are these?
        let sector = ((ch + sector_off()) as *const usize).read_unaligned();
        if sector != 0 && readable(sector, 0x80) {
            let arr = ((sector + 0x68) as *const usize).read_unaligned();
            let mut in_sector = 0;
            let mut sector_parts = 0;
            for i in 0..96usize {
                if arr == 0 || !readable(arr + i * 8, 8) {
                    break;
                }
                let e = ((arr + i * 8) as *const usize).read_unaligned();
                if e == 0 || e % 8 != 0 || !readable(e, 0x10) {
                    break;
                }
                if obj_class_name(e) == "TMotilePart" {
                    sector_parts += 1;
                    if parts.contains(&e) {
                        in_sector += 1;
                    }
                }
            }
            info_f!("Cruor TREE   sector list has {} body parts; {} of them are in this character's tree", sector_parts, in_sector);
        }
        // the other character in the sector, for contrast
        if let Some(pl) = (|| {
            let slot = PLAYER_SLOT.load(Ordering::Relaxed);
            if slot == 0 { None } else { Some(unsafe { (slot as *const usize).read_unaligned() }) }
        })() {
            if pl != ch && pl != 0 {
                let mut pp = Vec::new();
                body_parts_of(pl, &mut pp);
                let overlap = pp.iter().filter(|p| parts.contains(p)).count();
                info_f!(
                    "Cruor TREE   player 0x{:X}: tree gives {} parts; hit part {} theirs; shared with the hit character: {}",
                    pl, pp.len(), if pp.contains(&bone) { "IS" } else { "is NOT" }, overlap
                );
            }
        }
    }
}

/// TEST (first hit only): the sector list in order (index, class, address), and for each
/// character in it, any list among its fields (direct, or one object deep) that holds
/// body parts from the sector list - the character -> parts link.
fn probe_part_owners(ch: usize) {
    unsafe {
        let sector = ((ch + sector_off()) as *const usize).read_unaligned();
        if sector == 0 || !readable(sector, 0x80) {
            return;
        }
        let arr = ((sector + 0x68) as *const usize).read_unaligned();
        if arr == 0 {
            return;
        }
        // the list in order
        let mut ents: Vec<(usize, String)> = Vec::new();
        for i in 0..96usize {
            if !readable(arr + i * 8, 8) {
                break;
            }
            let e = ((arr + i * 8) as *const usize).read_unaligned();
            let c = if e != 0 && e % 8 == 0 && readable(e, 0x10) { obj_class_name(e) } else { String::new() };
            ents.push((e, c));
        }
        info_f!("Cruor OWNERS sector list (+0x68) in order, {} slots read:", ents.len());
        for chunk in ents.chunks(6).enumerate() {
            let (k, c) = chunk;
            let line: Vec<String> = c
                .iter()
                .enumerate()
                .map(|(j, (e, n))| format!("{}:{}@{:X}", k * 6 + j, if n.is_empty() { "-" } else { n.as_str() }, e))
                .collect();
            info_f!("Cruor OWNERS   {}", line.join("  "));
        }
        let parts: Vec<usize> = ents.iter().filter(|(_, n)| n == "TMotilePart").map(|(e, _)| *e).collect();
        let chars: Vec<usize> = ents.iter().filter(|(_, n)| n == "TMotileDynamics").map(|(e, _)| *e).collect();
        // each character: fields that hold several of these parts
        for c in chars {
            if !readable(c, 0x1400) {
                continue;
            }
            let mut reported = 0;
            let mut check = |base: usize, desc: String| {
                if reported >= 6 || base < 0x10000 || base % 8 != 0 || !readable(base, 8 * 64) {
                    return;
                }
                let mut hits = Vec::new();
                for i in 0..64usize {
                    let e = unsafe { ((base + i * 8) as *const usize).read_unaligned() };
                    if parts.contains(&e) {
                        hits.push(i);
                    }
                }
                if hits.len() >= 3 {
                    reported += 1;
                    info_f!("Cruor OWNERS   character 0x{:X}: {} -> {} of the sector's body parts, at elements {:?}", c, desc, hits.len(), hits);
                }
            };
            for off in (0..0x1400usize).step_by(8) {
                let p = ((c + off) as *const usize).read_unaligned();
                check(p, format!("+0x{:X}", off));
                if p > 0x10000 && p % 8 == 0 && readable(p, 0x100) {
                    for off2 in (0..0x100usize).step_by(8) {
                        let q = ((p + off2) as *const usize).read_unaligned();
                        check(q, format!("+0x{:X} -> ({}) +0x{:X}", off, obj_class_name(p), off2));
                    }
                }
            }
            if reported == 0 {
                info_f!("Cruor OWNERS   character 0x{:X}: no field holds 3+ of the sector's body parts (one object deep)", c);
            }
        }
        info_f!("Cruor OWNERS ===== end (hit character was 0x{:X}) =====", ch);
    }
}

/// TEST (first hit only): the body part's 4 tetrahedron corners (bone +0xB0, read by the
/// game's 0x10001CA40), its fields that point back to the character, and the contents
/// of the character's sector lists (+0x5F0 -> TSimSector), grouped by class and owner.
fn probe_parts(ch: usize, bone: usize) {
    unsafe {
        if !readable(bone, 0x400) || !readable(ch, 0x600) {
            info_f!("Cruor PARTS: part or character not readable");
            return;
        }
        let v = |i: usize| unsafe { rv3(bone + corners() + i * 12) };
        let (a, b, c, d) = (v(0), v(1), v(2), v(3));
        info_f!(
            "Cruor PARTS part 0x{:X} ({}): corners ({:.1},{:.1},{:.1}) ({:.1},{:.1},{:.1}) ({:.1},{:.1},{:.1}) ({:.1},{:.1},{:.1})",
            bone, obj_class_name(bone), a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2], d[0], d[1], d[2]
        );
        let mut owner_offs = Vec::new();
        for off in (0..0x400usize).step_by(8) {
            if ((bone + off) as *const usize).read_unaligned() == ch {
                owner_offs.push(off);
            }
        }
        info_f!("Cruor PARTS   part fields pointing back to the character: {:?}", owner_offs.iter().map(|o| format!("+0x{:X}", o)).collect::<Vec<_>>());
        let owner_off = owner_offs.first().copied();
        let sector = ((ch + sector_off()) as *const usize).read_unaligned();
        if sector == 0 || !readable(sector, 0x200) {
            info_f!("Cruor PARTS   sector 0x{:X} not readable", sector);
            return;
        }
        info_f!("Cruor PARTS   sector 0x{:X} ({})", sector, obj_class_name(sector));
        for arr_off in [0x68usize, 0xE8] {
            let arr = ((sector + arr_off) as *const usize).read_unaligned();
            let (n_before, n_after) = (
                ((sector + arr_off - 8) as *const i32).read_unaligned(),
                ((sector + arr_off + 8) as *const i32).read_unaligned(),
            );
            if arr == 0 || !readable(arr, 8) {
                info_f!("Cruor PARTS   sector +0x{:X}: no array", arr_off);
                continue;
            }
            // read until a null / unreadable / non-object entry, at most 256
            let mut classes: Vec<(String, u32)> = Vec::new();
            let mut owners: Vec<(usize, u32)> = Vec::new();
            let mut n = 0;
            let mut bone_at = None;
            for i in 0..256usize {
                if !readable(arr + i * 8, 8) {
                    break;
                }
                let e = ((arr + i * 8) as *const usize).read_unaligned();
                if e == 0 || e % 8 != 0 || !readable(e, 0x400) {
                    break;
                }
                let cname = obj_class_name(e);
                if cname.is_empty() || cname == "?" {
                    break;
                }
                n += 1;
                if e == bone {
                    bone_at = Some(i);
                }
                match classes.iter_mut().find(|(c, _)| *c == cname) {
                    Some(x) => x.1 += 1,
                    None => classes.push((cname.clone(), 1)),
                }
                if let Some(oo) = owner_off {
                    if cname == "TMotilePart" {
                        let o = ((e + oo) as *const usize).read_unaligned();
                        match owners.iter_mut().find(|(x, _)| *x == o) {
                            Some(x) => x.1 += 1,
                            None => owners.push((o, 1)),
                        }
                    }
                }
            }
            info_f!(
                "Cruor PARTS   sector +0x{:X}: {} objects read (count fields: before {}, after {}); hit part at index {:?}; classes {:?}",
                arr_off, n, n_before, n_after, bone_at, classes
            );
            if !owners.is_empty() {
                let list: Vec<String> = owners
                    .iter()
                    .map(|(o, k)| format!("0x{:X} ({}): {} parts", o, if *o != 0 && readable(*o, 8) { obj_class_name(*o) } else { "-".into() }, k))
                    .collect();
                info_f!("Cruor PARTS     body parts by owner (part +0x{:X}): {}", owner_off.unwrap_or(0), list.join(" | "));
            }
        }
    }
}
pub(crate) fn probe_bone_of(ch: usize, bone: usize) {
    if ch == 0 {
        return;
    }
    unsafe {
        if !readable(ch, 0x1400) {
            info_f!("Cruor BONE    character 0x{:X} not readable", ch);
            return;
        }
        let hr = ch + blood_arr();
        if bone == 0 || !readable(bone, 0xF0) {
            info_f!("Cruor BONE    bone 0x{:X} not readable", bone);
            return;
        }
        let n = BONE_PROBES.fetch_add(1, Ordering::Relaxed);
        let bm = |i: usize| unsafe { rf32(bone + corners() + i * 4) };
        let hip = read_hip(ch);
        info_f!(
            "Cruor BONE #{} character 0x{:X} ({}): hit bone 0x{:X} ({}); hit point in bone space ({:.1},{:.1},{:.1}); world ({:.1},{:.1},{:.1}); strength {:.3}; character hips ({:.1},{:.1},{:.1})",
            n, ch, obj_class_name(ch), bone, obj_class_name(bone),
            rf32(hr + 8), rf32(hr + 12), rf32(hr + 16), rf32(hr + 0x18), rf32(hr + 0x1C), rf32(hr + 0x20), rf32(hr + 0x28),
            hip[0], hip[1], hip[2]
        );
        info_f!(
            "Cruor BONE    bone matrix @+0xB0: [{:.3} {:.3} {:.3} {:.3}] [{:.3} {:.3} {:.3} {:.3}] [{:.3} {:.3} {:.3} {:.3}] [{:.1} {:.1} {:.1} {:.3}]",
            bm(0), bm(1), bm(2), bm(3), bm(4), bm(5), bm(6), bm(7), bm(8), bm(9), bm(10), bm(11), bm(12), bm(13), bm(14), bm(15)
        );
        // where does the character keep this bone?
        let mut found = 0;
        for off in (0..0x1400usize).step_by(8) {
            let p = ((ch + off) as *const usize).read_unaligned();
            if p == bone {
                info_f!("Cruor BONE    found: character +0x{:X} points straight at the bone", off);
                found += 1;
                continue;
            }
            if p < 0x10000 || p % 8 != 0 || !readable(p, 8 * 64) {
                continue;
            }
            // p as an array of pointers
            for i in 0..64usize {
                if ((p + i * 8) as *const usize).read_unaligned() == bone {
                    let len = if readable(p - 8, 8) { ((p - 8) as *const i64).read_unaligned() } else { -1 };
                    info_f!(
                        "Cruor BONE    found: character +0x{:X} -> array, element {} (length word before it: {}); elements are {}",
                        off, i, len, obj_class_name(((p) as *const usize).read_unaligned())
                    );
                    found += 1;
                    break;
                }
            }
            // p as an object holding an array (one level deeper)
            if found < 8 && readable(p, 0x200) {
                for off2 in (0..0x200usize).step_by(8) {
                    let q = ((p + off2) as *const usize).read_unaligned();
                    if q < 0x10000 || q % 8 != 0 || !readable(q, 8 * 64) {
                        continue;
                    }
                    for i in 0..64usize {
                        if ((q + i * 8) as *const usize).read_unaligned() == bone {
                            info_f!(
                                "Cruor BONE    found: character +0x{:X} -> object ({}) +0x{:X} -> array, element {}; count fields nearby: +0x{:X}={} +0x{:X}={}",
                                off, obj_class_name(p), off2, i,
                                off2.wrapping_sub(8), if off2 >= 8 { ((p + off2 - 8) as *const i32).read_unaligned() } else { 0 },
                                off2 + 8, ((p + off2 + 8) as *const i32).read_unaligned()
                            );
                            found += 1;
                            break;
                        }
                    }
                }
            }
            if found >= 8 {
                break;
            }
        }
        if found == 0 {
            info_f!("Cruor BONE    not found in the character's first 0x1400 bytes (one or two pointers deep)");
        }
    }
}

/// TEST: dump the player's model the way the game's hit code walks it:
/// character +0x12F0 -> +0x18 = model; parts: count +0x118, array +0x120.
fn character_model_probe() {
    let slot = PLAYER_SLOT.load(Ordering::Relaxed);
    if slot == 0 {
        info_f!("Cruor PROBE character: player not found");
        return;
    }
    unsafe {
        let ch = (slot as *const usize).read_unaligned();
        if !readable(ch, 0x1300) {
            info_f!("Cruor PROBE character: player 0x{:X} not readable", ch);
            return;
        }
        let a = ((ch + items_off()) as *const usize).read_unaligned();
        info_f!("Cruor PROBE character ===== player 0x{:X} ({}), +0x12F0 = 0x{:X} =====", ch, obj_class_name(ch), a);
        // the character's pending hit record (+0x8A0): world position @+0x18, amount @+0x28
        let hr = ch + blood_arr();
        info_f!(
            "Cruor PROBE   hit record @+0x8A0: first 0x{:X}; local ({:.1},{:.1},{:.1}); world ({:.1},{:.1},{:.1}); +0x24 {}; amount {:.3}",
            (hr as *const usize).read_unaligned(), rf32(hr + 8), rf32(hr + 12), rf32(hr + 16),
            rf32(hr + 0x18), rf32(hr + 0x1C), rf32(hr + 0x20), ((hr + 0x24) as *const i32).read_unaligned(), rf32(hr + 0x28)
        );
        if a == 0 || !readable(a, 0x20) {
            return;
        }
        let model = ((a + 0x18) as *const usize).read_unaligned();
        if model == 0 || !readable(model, 0x128) {
            info_f!("Cruor PROBE   model 0x{:X} not readable", model);
            return;
        }
        let hi = ((model + 0x118) as *const i32).read_unaligned();
        let arr = ((model + 0x120) as *const usize).read_unaligned();
        info_f!("Cruor PROBE   model 0x{:X} ({}): part list hi index {}, array 0x{:X}", model, obj_class_name(model), hi, arr);
        if !(0..512).contains(&hi) || arr == 0 || !readable(arr, (hi as usize + 1) * 8) {
            return;
        }
        for i in 0..=(hi as usize) {
            let part = ((arr + i * 8) as *const usize).read_unaligned();
            if part == 0 || !readable(part, 0x170) {
                info_f!("Cruor PROBE   part {}: 0x{:X} (not readable)", i, part);
                continue;
            }
            let ty = (part as *const i32).add(2).read_unaligned();
            let fl = ((part + 0xC) as *const u32).read_unaligned();
            let rbd = ((part + 0x160) as *const usize).read_unaligned();
            let rbd_kind = if rbd != 0 && readable(rbd, 0x10) { ((rbd + 0xC) as *const u32).read_unaligned() } else { 0 };
            let c = rv3(part + 0x20);
            let t = rv3(part + 0x64);
            info_f!(
                "Cruor PROBE   part {}: 0x{:X} {} type 0x{:X} flags 0x{:08X} mesh {}; rigid body 0x{:X} {} kind 0x{:08X}; sphere centre ({:.1},{:.1},{:.1}) r {:.1}; offset ({:.1},{:.1},{:.1})",
                i, part, obj_class_name(part), ty, fl, ty & 0x89 == 0x89,
                rbd, if rbd != 0 && readable(rbd, 8) { obj_class_name(rbd) } else { "-".into() }, rbd_kind,
                c[0], c[1], c[2], rf32(part + 0x2C), t[0], t[1], t[2]
            );
        }
        info_f!("Cruor PROBE character ===== end =====");
    }
}

/// STAIRS PROBE (F9): cast a vertical line through the player's spot and log every
/// piece it passes through (height vs the player's feet, surface angle, kind, flags),
/// plus pieces around that aren't used for collision, and why.
static PROBE_REQ: AtomicBool = AtomicBool::new(false);
fn stairs_probe() {
    let hip = match player_hips() {
        Some(h) => h,
        None => {
            warn_f!("Cruor PROBE: player not found");
            return;
        }
    };
    let feet = hip[1] - HIP_HEIGHT;
    let p0 = [hip[0], hip[1] + 150.0, hip[2]];
    let p1 = [hip[0], hip[1] - 300.0, hip[2]];
    let d = sub3(p1, p0);
    let g = match LEVEL.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let idx = match g.as_ref() {
        Some(i) => i,
        None => {
            warn_f!("Cruor PROBE: level collision not read");
            return;
        }
    };
    info_f!("Cruor PROBE ===== at x {:.0}, z {:.0}; hips y {:.0}, feet about {:.0} =====", hip[0], hip[2], hip[1], feet);
    let describe = |part: usize| -> String {
        unsafe {
            if !readable(part, 0x40) {
                return "?".into();
            }
            let ty = (part as *const i32).add(2).read_unaligned();
            let fl = ((part + 0xC) as *const u32).read_unaligned();
            format!("piece 0x{:X} type 0x{:X} flags 0x{:08X} {}", part, ty, fl, obj_class_name(part))
        }
    };
    let mut lines = 0;
    let mut test = |p: &PartRef, kind: &str| {
        let off = unsafe { rv3(p.obj + 0x64) };
        let c = unsafe { rv3(p.part + 0x20) };
        let cw = [off[0] + c[0], off[1] + c[1], off[2] + c[2]];
        if !seg_hits_sphere(p0, d, cw, p.radius) {
            return;
        }
        match unsafe { ray_part(p, p0, d, 1.0) } {
            Some((t, tri)) => {
                let y = p0[1] + d[1] * t;
                let n = unsafe { tri_normal(p, tri, d) }.unwrap_or([0.0, 0.0, 0.0]);
                info_f!(
                    "Cruor PROBE   HIT {} at y {:.1} ({:+.1} from feet), surface up-ness {:.2}: {}",
                    kind, y, y - feet, n[1], describe(p.part)
                );
            }
            None => {
                if lines < 12 {
                    info_f!("Cruor PROBE   (passes its bounding sphere, no triangle hit) {}: {}", kind, describe(p.part));
                }
            }
        }
        lines += 1;
    };
    let (cx, cz) = ((hip[0] / COL_CELL).floor() as i32, (hip[2] / COL_CELL).floor() as i32);
    if let Some(list) = idx.cells.get(&(cx, cz)) {
        for &k in list {
            test(&idx.stat[k as usize], "level");
        }
    }
    for &k in idx.big.iter() {
        test(&idx.stat[k as usize], "level(large)");
    }
    for p in idx.props.iter() {
        test(p, "prop");
    }
    // pieces NOT used for collision whose bounding sphere covers this spot
    let mut shown = 0;
    for (part, obj, reason) in idx.skipped.iter() {
        unsafe {
            if !readable(*part, 0x30) || !readable(*obj, 0x70) {
                continue;
            }
            let off = rv3(*obj + 0x64);
            let c = rv3(*part + 0x20);
            let r = rf32(*part + 0x2C);
            if !(r.is_finite() && r > 0.0 && r < 100000.0) {
                continue;
            }
            let cw = [off[0] + c[0], off[1] + c[1], off[2] + c[2]];
            if seg_hits_sphere(p0, d, cw, r) && shown < 20 {
                shown += 1;
                info_f!(
                    "Cruor PROBE   SKIPPED ({}) centre y {:.0}, radius {:.0}: {}",
                    match reason { 1 => "not a mesh type", 2 => "special kind (tested differently by the game)", _ => "no collision function" },
                    cw[1], r, describe(*part)
                );
            }
        }
    }
    info_f!("Cruor PROBE ===== end =====");
}

/// Where margin a prop may drift between grid rebuilds (cm).
const PROP_MARGIN: f32 = 40.0;

/// Re-sort the props into their grid by where they are now (every few frames).
fn refresh_prop_grid(l: &mut LevelIndex) {
    l.pcells.values_mut().for_each(|v| v.clear());
    l.pbig.clear();
    l.ppos.resize(l.props.len(), ([0.0; 3], 0.0));
    l.pstamp.resize(l.props.len(), 0);
    for (k, p) in l.props.iter().enumerate() {
        let alive = unsafe { (p.part as *const usize).read_unaligned() == p.vmt };
        if !alive {
            l.ppos[k] = ([0.0; 3], 0.0);
            continue;
        }
        let (off, c) = unsafe { (rv3(p.obj + 0x64), rv3(p.part + 0x20)) };
        let cw = [off[0] + c[0], off[1] + c[1], off[2] + c[2]];
        if !cw.iter().all(|v| v.is_finite()) {
            l.ppos[k] = ([0.0; 3], 0.0);
            continue;
        }
        l.ppos[k] = (cw, p.radius);
        let r = p.radius + PROP_MARGIN;
        if r > 400.0 {
            l.pbig.push(k as u32);
            continue;
        }
        let (x0, x1) = (((cw[0] - r) / COL_CELL).floor() as i32, ((cw[0] + r) / COL_CELL).floor() as i32);
        let (z0, z1) = (((cw[2] - r) / COL_CELL).floor() as i32, ((cw[2] + r) / COL_CELL).floor() as i32);
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                l.pcells.entry((cx, cz)).or_default().push(k as u32);
            }
        }
    }
    // drop empty squares now and then so the map doesn't grow forever
    if l.pcells.len() > 20000 {
        l.pcells.retain(|_, v| !v.is_empty());
    }
}

/// Keep the index current: full rebuild on a level change (or if the game's object
/// list moved), otherwise just add objects that appeared (dropped items etc.).
/// The game is using the current level (its world collision has run since the last
/// scene clear).
pub(crate) static WORLD_LIVE: AtomicBool = AtomicBool::new(false);

/// The game is clearing its scene (level unload / reload): the blood and our copy of
/// the level's collision go with it. Called from the hook on the game's scene clear.
pub(crate) fn on_scene_cleared(scene: usize) {
    forget_regions();
    forget_bodies();
    forget_geoms();
    forget_body_queue();
    WORLD_LIVE.store(false, Ordering::Relaxed);
    forget_cast_off();
    forget_spectate();
    if scene != 0 {
        SCENE.store(scene, Ordering::Relaxed);
    }
    if let Ok(mut b) = LEVEL_BUILD.lock() {
        *b = None;
    }
    if let Ok(mut g) = LEVEL.lock() {
        *g = None;
    }
    clear_blood("level unloaded");
}

/// A collision re-read in progress, done a few milliseconds per frame.
struct LevelBuild {
    idx: LevelIndex,
    next: usize,
    code_ok: std::collections::HashMap<usize, bool>,
    skipped: usize,
    started: std::time::Instant,
    frames: u32,
}
static LEVEL_BUILD: Mutex<Option<LevelBuild>> = Mutex::new(None);
/// Time per frame spent re-reading the level's collision.
const BUILD_BUDGET_MS: f64 = 3.0;

/// Start (or restart) re-reading the level's collision for this object list.
fn start_level_build(arr: usize, hi: i32) {
    set_stage(12);
    if !(0..200000).contains(&hi) || arr == 0 || !unsafe { readable(arr, (hi as usize + 1) * 8) } {
        return;
    }
    let idx = LevelIndex {
        arr, hi, stat: Vec::new(), stamp: Vec::new(), query: 0,
        cells: Default::default(), big: Vec::new(), props: Vec::new(),
        pcells: Default::default(), pbig: Vec::new(), ppos: Vec::new(), pstamp: Vec::new(), pframe: 0,
        skipped: Vec::new(),
        static_keys: Default::default(),
    };
    if let Ok(mut b) = LEVEL_BUILD.lock() {
        *b = Some(LevelBuild {
            idx, next: 0, code_ok: Default::default(), skipped: 0,
            started: std::time::Instant::now(), frames: 0,
        });
    }
}

/// Keep our copy of the level's collision in step with the game, never stalling a frame:
/// - after the game clears its scene, nothing until the game is using the new level
///   (its world collision runs), then it's read a few milliseconds per frame;
/// - objects the game adds are added straight away; objects it removes trigger a
///   re-read in slices while the current copy stays in use.
fn update_level_index() {
    let scene = SCENE.load(Ordering::Relaxed);
    if scene == 0 {
        return;
    }
    let (hi, arr) = match unsafe { (rd_i32(scene + 0x3C4), rd_usize(scene + 0x3C8)) } {
        (Some(h), Some(a)) => (h, a),
        _ => return,
    };

    // --- a read in progress: continue it (restart if the game moved its list) ---
    let building = LEVEL_BUILD.lock().map(|b| b.as_ref().map(|b| b.idx.arr)).unwrap_or(None);
    if let Some(barr) = building {
        if barr != arr {
            start_level_build(arr, hi);
            return;
        }
        let t0 = std::time::Instant::now();
        let mut finished: Option<LevelBuild> = None;
        set_stage(11);
        if let Ok(mut g) = LEVEL_BUILD.lock() {
            if let Some(b) = g.as_mut() {
                b.frames += 1;
                let last = b.idx.hi as usize;
                // nothing to collide with meanwhile (new level): read faster, still in slices
                let have_level = LEVEL.try_lock().map(|g| g.is_some()).unwrap_or(true);
                let budget = if have_level { BUILD_BUDGET_MS } else { 12.0 };
                while b.next <= last && t0.elapsed().as_secs_f64() * 1000.0 < budget {
                    let i = b.next;
                    b.skipped += unsafe { index_objects(&mut b.idx, i, i, &mut b.code_ok) };
                    b.next += 1;
                }
                if b.next > last {
                    finished = g.take();
                }
            }
        }
        if let Some(mut b) = finished {
            refresh_prop_grid(&mut b.idx);
            info_f!(
                "Cruor: level collision ready - {} level pieces, {} loose props, {} large pieces, {} skipped ({} ms over {} frames, no stall)",
                b.idx.stat.len(), b.idx.props.len(), b.idx.big.len(), b.skipped,
                b.started.elapsed().as_millis(), b.frames
            );
            if let Ok(mut g) = LEVEL.lock() {
                *g = Some(b.idx);
            }
        }
        return;
    }

    set_stage(15);
    let mut g = match LEVEL.try_lock() {
        Ok(g) => g,
        Err(_) => return, // busy: try again next frame, never wait
    };
    set_stage(10);
    // --- no copy yet: start reading once the game is using the level ---
    if g.is_none() {
        drop(g);
        if WORLD_LIVE.load(Ordering::Relaxed) {
            start_level_build(arr, hi);
        }
        return;
    }
    let l = g.as_mut().unwrap();
    // the game moved its object list in memory (it grew): same objects, new address
    if l.arr != arr {
        l.arr = arr;
    }
    // the game removed objects: re-read in slices, keep using this copy meanwhile
    if hi < l.hi {
        drop(g);
        start_level_build(arr, hi);
        return;
    }
    // the game added objects (dropped items...): add them now
    // (in slices of at most 3 ms a frame: the game can add thousands at once while it
    //  streams an area in)
    let mut grew = false;
    if hi > l.hi && unsafe { readable(arr, (hi as usize + 1) * 8) } {
        set_stage(13);
        let t0 = std::time::Instant::now();
        let mut code_ok = std::collections::HashMap::new();
        let mut i = (l.hi + 1) as usize;
        while i <= hi as usize && t0.elapsed().as_secs_f64() * 1000.0 < 3.0 {
            unsafe { index_objects(l, i, i, &mut code_ok) };
            l.hi = i as i32;
            i += 1;
            grew = true;
        }
    }
    l.pframe = l.pframe.wrapping_add(1);
    // loose props' positions only matter while blood is in the air
    let flying = !LAST_VERTS.lock().map(|v| v.is_empty()).unwrap_or(true);
    if grew || l.ppos.len() != l.props.len() || (flying && l.pframe % 3 == 0) {
        set_stage(14);
        refresh_prop_grid(l);
    }
}

static ROUND: AtomicU32 = AtomicU32::new(0);
/// Collision cost, reported every 5 seconds: (tests, total time, worst frame, frames).
static RAY_COST: Mutex<(u64, f64, f64, u32, Option<std::time::Instant>)> = Mutex::new((0, 0.0, 0.0, 0, None));
fn note_ray_cost(tests: usize, took: std::time::Duration) {
    let ms = took.as_secs_f64() * 1000.0;
    if let Ok(mut c) = RAY_COST.lock() {
        c.0 += tests as u64;
        c.1 += ms;
        c.2 = c.2.max(ms);
        c.3 += 1;
        let now = std::time::Instant::now();
        let start = *c.4.get_or_insert(now);
        if (now - start).as_secs_f32() >= 5.0 {
            if c.0 > 0 {
                if !RELEASE { info_f!(
                    "Cruor: collision - {} tests in 5 s, {:.2} ms per frame on average, worst frame {:.2} ms; landed: floor {}, wall {}, prop {}, character {} (bodies known: {} characters, {} parts)",
                    c.0, c.1 / c.3.max(1) as f64, c.2,
                    COL_FLOOR.swap(0, Ordering::Relaxed), COL_WALL.swap(0, Ordering::Relaxed), COL_PROP.swap(0, Ordering::Relaxed),
                    COL_BODY.swap(0, Ordering::Relaxed), BODY_CHARS.load(Ordering::Relaxed), BODY_PARTS.load(Ordering::Relaxed)
                ); }
            }
            *c = (0, 0.0, 0.0, 0, Some(now));
        }
    }
}

struct Hit {
    pos: [f32; 3],
    normal: [f32; 3],
    prop: Option<PartRef>,
}

/// Ray-test one model along p0 -> p1 using the game's own collision function.
/// Returns the fraction along the segment and the triangle index.
/// THE PLUGIN'S OWN RAY TEST on the game's triangles.
/// The game's ray functions (+0x3B0: 0x1000605A0, or the kd-tree 0x100269020) are NOT
/// called: the kd-tree one walks a stack in global memory (0x1005B2470) that the game's
/// own simulation thread uses too - calling it from here corrupted both (freezes,
/// crashes, the game's simulation errors). Instead each part's triangles are copied
/// once, exactly as the game's 0x1000605A0 walks them: sub-meshes (hi +0x1C8, array
/// +0x1D0), each covering triangles [sub+0xB0 .. sub+0xB4]; triangles at +0x2A8, 0x30
/// bytes each, vertex pointers at +0x10/+0x18/+0x20 (model-local positions). The test is
/// the game's: one-sided, det >= 1.1920899e-7, u,v >= 0, nearest t.
struct Geom {
    tris: Vec<[f32; 9]>,
    idx: Vec<i32>,
    min: [f32; 3],
    inv_cell: [f32; 3],
    n: [usize; 3],
    cells: Vec<Vec<u32>>,
}
static GEOMS: Mutex<Option<std::collections::HashMap<usize, Option<std::sync::Arc<Geom>>>>> = Mutex::new(None);
fn forget_geoms() {
    if let Ok(mut g) = GEOMS.lock() {
        *g = None;
    }
}
unsafe fn build_geom(part: usize) -> Option<Geom> {
    unsafe {
        if !readable(part, 0x2B0) {
            return None;
        }
        let hi = ((part + 0x1C8) as *const i32).read_unaligned();
        let subs = ((part + 0x1D0) as *const usize).read_unaligned();
        let tarr = ((part + 0x2A8) as *const usize).read_unaligned();
        if !(0..4096).contains(&hi) || subs == 0 || tarr == 0 || !readable(subs, (hi as usize + 1) * 8) {
            return None;
        }
        let mut tris: Vec<[f32; 9]> = Vec::new();
        let mut idx: Vec<i32> = Vec::new();
        for si in 0..=(hi as usize) {
            let sub = ((subs + si * 8) as *const usize).read_unaligned();
            if sub == 0 || !readable(sub, 0xB8) {
                continue;
            }
            let first = ((sub + 0xB0) as *const i32).read_unaligned();
            let last = ((sub + 0xB4) as *const i32).read_unaligned();
            if first < 0 || last < first || last - first > 200_000 {
                continue;
            }
            for ti in first..=last {
                if tris.len() >= 400_000 {
                    break;
                }
                let rec = tarr + ti as usize * 0x30;
                if !readable(rec, 0x30) {
                    break;
                }
                let vp = [
                    ((rec + 0x10) as *const usize).read_unaligned(),
                    ((rec + 0x18) as *const usize).read_unaligned(),
                    ((rec + 0x20) as *const usize).read_unaligned(),
                ];
                if vp.iter().any(|&a| a < 0x10000 || !readable(a, 12)) {
                    continue;
                }
                let (a, b, c) = (rv3(vp[0]), rv3(vp[1]), rv3(vp[2]));
                if !a.iter().chain(b.iter()).chain(c.iter()).all(|v| v.is_finite()) {
                    continue;
                }
                tris.push([a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]]);
                idx.push(ti);
            }
        }
        if tris.is_empty() {
            return None;
        }
        // a coarse grid over the part (about 8 triangles a cell, at most 32 a side)
        let mut mn = [f32::INFINITY; 3];
        let mut mx = [f32::NEG_INFINITY; 3];
        for t in tris.iter() {
            for v in 0..3 {
                for a in 0..3 {
                    mn[a] = mn[a].min(t[v * 3 + a]);
                    mx[a] = mx[a].max(t[v * 3 + a]);
                }
            }
        }
        let ext = [(mx[0] - mn[0]).max(1.0), (mx[1] - mn[1]).max(1.0), (mx[2] - mn[2]).max(1.0)];
        let target = ((tris.len() as f32 / 8.0).max(1.0)).cbrt();
        let mean = (ext[0] * ext[1] * ext[2]).cbrt();
        let n = [
            ((ext[0] / mean * target).ceil() as usize).clamp(1, 32),
            ((ext[1] / mean * target).ceil() as usize).clamp(1, 32),
            ((ext[2] / mean * target).ceil() as usize).clamp(1, 32),
        ];
        let inv_cell = [n[0] as f32 / ext[0], n[1] as f32 / ext[1], n[2] as f32 / ext[2]];
        let mut cells = vec![Vec::new(); n[0] * n[1] * n[2]];
        let ci = |v: f32, a: usize| -> usize { (((v - mn[a]) * inv_cell[a]).floor().max(0.0) as usize).min(n[a] - 1) };
        for (k, t) in tris.iter().enumerate() {
            let mut lo = [usize::MAX; 3];
            let mut up = [0usize; 3];
            for a in 0..3 {
                for v in 0..3 {
                    let c = ci(t[v * 3 + a], a);
                    lo[a] = lo[a].min(c);
                    up[a] = up[a].max(c);
                }
            }
            for x in lo[0]..=up[0] {
                for y in lo[1]..=up[1] {
                    for z in lo[2]..=up[2] {
                        cells[(z * n[1] + y) * n[0] + x].push(k as u32);
                    }
                }
            }
        }
        Some(Geom { tris, idx, min: mn, inv_cell, n, cells })
    }
}
fn geom_for(part: usize) -> Option<std::sync::Arc<Geom>> {
    let mut g = GEOMS.lock().ok()?;
    let map = g.get_or_insert_with(Default::default);
    if let Some(e) = map.get(&part) {
        return e.clone();
    }
    let built = unsafe { build_geom(part) }.map(std::sync::Arc::new);
    if map.len() > 60_000 {
        map.clear();
    }
    map.insert(part, built.clone());
    built
}

unsafe fn ray_part(p: &PartRef, p0: [f32; 3], d: [f32; 3], best: f32) -> Option<(f32, i32)> {
    unsafe {
        if (p.part as *const usize).read_unaligned() != p.vmt {
            return None; // model no longer there
        }
        let (r0, r1, r2, t) = part_pose(p);
        let rel = sub3(p0, t);
        let s0 = [dot3(r0, rel), dot3(r1, rel), dot3(r2, rel)];
        let dl = [dot3(r0, d), dot3(r1, d), dot3(r2, d)];
        let geom = geom_for(p.part)?;
        ray_geom(&geom, s0, dl, best)
    }
}

/// The triangle test in the model's own coordinates: segment s0 .. s0 + dl*best.
unsafe fn ray_geom(geom: &Geom, s0: [f32; 3], dl: [f32; 3], best: f32) -> Option<(f32, i32)> {
    unsafe {
        // cells the segment s0 .. s0 + dl*best passes over (drop paths are short)
        let e = [s0[0] + dl[0] * best, s0[1] + dl[1] * best, s0[2] + dl[2] * best];
        let mut lo = [0usize; 3];
        let mut up = [0usize; 3];
        for a in 0..3 {
            let (amin, amax) = (s0[a].min(e[a]), s0[a].max(e[a]));
            let cmin = ((amin - geom.min[a]) * geom.inv_cell[a]).floor();
            let cmax = ((amax - geom.min[a]) * geom.inv_cell[a]).floor();
            if cmax < 0.0 || cmin >= geom.n[a] as f32 {
                return None; // misses the part's triangles entirely
            }
            lo[a] = cmin.max(0.0) as usize;
            up[a] = (cmax as usize).min(geom.n[a] - 1);
        }
        let mut best_t = best;
        let mut hit: Option<i32> = None;
        for x in lo[0]..=up[0] {
            for y in lo[1]..=up[1] {
                for z in lo[2]..=up[2] {
                    for &k in geom.cells[(z * geom.n[1] + y) * geom.n[0] + x].iter() {
                        let tr = &geom.tris[k as usize];
                        let v0 = [tr[0], tr[1], tr[2]];
                        let e1 = [tr[3] - tr[0], tr[4] - tr[1], tr[5] - tr[2]];
                        let e2 = [tr[6] - tr[0], tr[7] - tr[1], tr[8] - tr[2]];
                        let pv = cross3(dl, e2);
                        let det = dot3(pv, e1);
                        if det < 1.1920899e-7 {
                            continue; // (one-sided, as the game's test)
                        }
                        let tv = sub3(s0, v0);
                        let u = dot3(tv, pv);
                        if u < 0.0 || u > det {
                            continue;
                        }
                        let q = cross3(tv, e1);
                        let v = dot3(dl, q);
                        if v < 0.0 || u + v > det {
                            continue;
                        }
                        let tt = dot3(e2, q) / det;
                        if tt > 0.0 && tt < best_t {
                            best_t = tt;
                            hit = Some(geom.idx[k as usize]);
                        }
                    }
                }
            }
        }
        hit.map(|tri| (best_t, tri))
    }
}

/// Surface direction of the triangle that was hit, in world space (from the copy).
unsafe fn tri_normal(p: &PartRef, tri: i32, d: [f32; 3]) -> Option<[f32; 3]> {
    unsafe {
        let geom = geom_for(p.part)?;
        let k = geom.idx.iter().position(|&i| i == tri)?;
        let tr = &geom.tris[k];
        let (v0, v1, v2) = ([tr[0], tr[1], tr[2]], [tr[3], tr[4], tr[5]], [tr[6], tr[7], tr[8]]);
        let nl = cross3(sub3(v1, v0), sub3(v2, v0));
        let (r0, r1, r2, _) = part_pose(p);
        let mut n = norm3([
            nl[0] * r0[0] + nl[1] * r1[0] + nl[2] * r2[0],
            nl[0] * r0[1] + nl[1] * r1[1] + nl[2] * r2[1],
            nl[0] * r0[2] + nl[1] * r1[2] + nl[2] * r2[2],
        ]);
        if dot3(n, d) > 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        if n == [0.0, 0.0, 0.0] { None } else { Some(n) }
    }
}

fn seg_hits_sphere(p0: [f32; 3], d: [f32; 3], c: [f32; 3], r: f32) -> bool {
    let len2 = dot3(d, d).max(1e-6);
    let t = (dot3(sub3(c, p0), d) / len2).clamp(0.0, 1.0);
    let q = [p0[0] + d[0] * t, p0[1] + d[1] * t, p0[2] + d[2] * t];
    let e = sub3(q, c);
    dot3(e, e) <= r * r
}

/// Where does the path p0 -> p1 first hit the level or a prop? (None = nothing in the way)
fn game_raycast(p0: [f32; 3], p1: [f32; 3]) -> Option<Hit> {
    let d = sub3(p1, p0);
    if dot3(d, d) < 1e-6 {
        return None;
    }
    let mut g = LEVEL.lock().ok()?;
    let idx = g.as_mut()?;
    idx.query = idx.query.wrapping_add(1);
    let q = idx.query;
    let mut best = 1.0f32;
    let mut hit: Option<(PartRef, i32, bool)> = None;
    unsafe {
        // level pieces in the grid squares the path crosses
        let (x0, x1) = ((p0[0].min(p1[0]) / COL_CELL).floor() as i32, (p0[0].max(p1[0]) / COL_CELL).floor() as i32);
        let (z0, z1) = ((p0[2].min(p1[2]) / COL_CELL).floor() as i32, (p0[2].max(p1[2]) / COL_CELL).floor() as i32);
        for cx in x0..=x1.min(x0 + 3) {
            for cz in z0..=z1.min(z0 + 3) {
                if let Some(list) = idx.cells.get(&(cx, cz)) {
                    for &k in list {
                        if idx.stamp[k as usize] == q {
                            continue; // already checked for this path
                        }
                        idx.stamp[k as usize] = q;
                        let p = &idx.stat[k as usize];
                        if !seg_hits_sphere(p0, d, p.center, p.radius) {
                            continue;
                        }
                        if let Some((t, tri)) = ray_part(p, p0, d, best) {
                            best = t;
                            hit = Some((*p, tri, false));
                        }
                    }
                }
            }
        }
        for &k in idx.big.iter() {
            let p = &idx.stat[k as usize];
            if !seg_hits_sphere(p0, d, p.center, p.radius) {
                continue;
            }
            if let Some((t, tri)) = ray_part(p, p0, d, best) {
                best = t;
                hit = Some((*p, tri, false));
            }
        }
        // props near the path (grid of where they are now, with a drift margin)
        let mut cand: Vec<u32> = Vec::new();
        for cx in x0..=x1.min(x0 + 3) {
            for cz in z0..=z1.min(z0 + 3) {
                if let Some(list) = idx.pcells.get(&(cx, cz)) {
                    for &k in list {
                        if idx.pstamp[k as usize] != q {
                            idx.pstamp[k as usize] = q;
                            cand.push(k);
                        }
                    }
                }
            }
        }
        cand.extend(idx.pbig.iter().copied());
        for k in cand {
            let (c, r) = idx.ppos[k as usize];
            if r <= 0.0 || !seg_hits_sphere(p0, d, c, r + PROP_MARGIN) {
                continue;
            }
            let p = idx.props[k as usize];
            if let Some((t, tri)) = ray_part(&p, p0, d, best) {
                best = t;
                hit = Some((p, tri, true));
            }
        }
        let (p, tri, is_prop) = hit?;
        let pos = [p0[0] + d[0] * best, p0[1] + d[1] * best, p0[2] + d[2] * best];
        let normal = tri_normal(&p, tri, d).unwrap_or_else(|| {
            let n = norm3(d);
            [-n[0], -n[1], -n[2]]
        });
        Some(Hit { pos, normal, prop: if is_prop { Some(p) } else { None } })
    }
}

// ---- Blood stuck to props (by the exact model that was hit) ----
const MAX_PROP_SPLATS: usize = 32;
static PROP_SPLATS: Mutex<Vec<(PartRef, ObjBlood)>> = Mutex::new(Vec::new());
/// Snapshot each frame: (world offset, first rotation row, splats) for matching draws.
static PROP_FRAME: Mutex<Vec<([f32; 3], [f32; 3], Vec<f32>)>> = Mutex::new(Vec::new());

/// OBJECT BLOOD MAPS: one shared atlas; each bloodied object gets a slot of three
/// OBJ_TILE x OBJ_TILE tiles (one per side direction in its own coordinates), sized from
/// the game's own bounding sphere of the part (+0x20 centre, +0x2C radius). Up to
/// ATLAS_SLOTS objects; the longest-unbled gives up its slot when more are needed.
const ATLAS_N: usize = 4096;
const OBJ_TILE: usize = 256;
const ATLAS_COLS: usize = ATLAS_N / (OBJ_TILE * 3); // 5
const ATLAS_SLOTS: usize = ATLAS_COLS * (ATLAS_N / OBJ_TILE); // 80
struct ObjAtlas {
    data: Vec<f32>,
    dirty: Dirty,
    allocated: bool,
}
static ATLAS: Mutex<Option<ObjAtlas>> = Mutex::new(None);
#[derive(Clone, Copy)]
struct ObjBlood {
    slot: usize,
    c: [f32; 3],
    inv_r: f32,
    last: u32,
}
fn slot_origin(slot: usize) -> (usize, usize) {
    ((slot % ATLAS_COLS) * OBJ_TILE * 3, (slot / ATLAS_COLS) * OBJ_TILE)
}
fn atlas_clear_slot(a: &mut ObjAtlas, slot: usize) {
    let (x0, y0) = slot_origin(slot);
    for y in y0..y0 + OBJ_TILE {
        for x in x0..x0 + OBJ_TILE * 3 {
            if a.data[y * ATLAS_N + x] != 0.0 {
                a.data[y * ATLAS_N + x] = 0.0;
                a.dirty.mark(x, y);
            }
        }
    }
}
/// The atlas texture is cleared on the card next frame (instead of uploading 32 MB of zeros).
static ATLAS_GPU_CLEAR: AtomicBool = AtomicBool::new(false);
fn atlas_clear_all() {
    if let Ok(mut g) = ATLAS.lock() {
        if let Some(a) = g.as_mut() {
            a.data.iter_mut().for_each(|v| *v = 0.0);
            a.dirty = Dirty::new(ATLAS_N, ATLAS_N);
            ATLAS_GPU_CLEAR.store(true, Ordering::Relaxed);
        }
    }
}
/// Paint one splat into an object's tile for the side it hit.
fn atlas_paint(a: &mut ObjAtlas, ob: &ObjBlood, local: [f32; 3], ln: [f32; 3], rad: f32) {
    let ax = if ln[0].abs() >= ln[1].abs() && ln[0].abs() >= ln[2].abs() { 0 } else if ln[1].abs() >= ln[2].abs() { 1 } else { 2 };
    let q = [(local[0] - ob.c[0]) * ob.inv_r, (local[1] - ob.c[1]) * ob.inv_r, (local[2] - ob.c[2]) * ob.inv_r];
    let (tu, tv) = match ax { 0 => (q[1], q[2]), 1 => (q[0], q[2]), _ => (q[0], q[1]) };
    let (sx, sy) = slot_origin(ob.slot);
    let half = OBJ_TILE as f32 * 0.5;
    let cx = sx as f32 + ax as f32 * OBJ_TILE as f32 + (tu * 0.5 + 0.5) * OBJ_TILE as f32;
    let cy = sy as f32 + (tv * 0.5 + 0.5) * OBJ_TILE as f32;
    let r = (rad * ob.inv_r * half).max(0.8); // texels
    let e = (r * 0.25).max(1.5);
    let seed = ((local[0] * 12.9898 + local[2] * 78.233).sin() * 43758.547).fract().abs();
    let ph3 = ((seed * 20.0).cos(), (seed * 20.0).sin());
    let ph5 = ((seed * 13.0).cos(), (seed * 13.0).sin());
    let tile_x0 = (sx + ax * OBJ_TILE) as f32;
    let (x0, x1) = (((cx - r - e - 1.0).floor().max(tile_x0)) as usize, ((cx + r + e + 1.0).ceil().min(tile_x0 + OBJ_TILE as f32 - 1.0)) as usize);
    let (y0, y1) = (((cy - r - e - 1.0).floor().max(sy as f32)) as usize, ((cy + r + e + 1.0).ceil().min((sy + OBJ_TILE - 1) as f32)) as usize);
    if x0 > x1 || y0 > y1 {
        return;
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d0 = (dx * dx + dy * dy).sqrt();
            let z = if d0 > 1e-6 { (dx / d0, dy / d0) } else { (1.0, 0.0) };
            let z2 = cmul(z, z);
            let z3 = cmul(z2, z);
            let z5 = cmul(z3, z2);
            let rim = r * (0.86 + 0.07 * wobble(z3, ph3) + 0.05 * wobble(z5, ph5));
            let amt = 1.0 - smooth((rim - e).max(0.0), rim + 0.5 * e, d0);
            let k = y * ATLAS_N + x;
            if amt > a.data[k] {
                a.data[k] = amt;
                a.dirty.mark(x, y);
            }
        }
    }
}
/// Upload the changed parts of the atlas (16-bit, like the maps).
unsafe fn update_obj_atlas(gl: &Gl, tex: u32) {
    let mut g = match ATLAS.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let a = g.get_or_insert_with(|| ObjAtlas { data: vec![0.0; ATLAS_N * ATLAS_N], dirty: Dirty::new(ATLAS_N, ATLAS_N), allocated: false });
    if a.allocated && !a.dirty.any && !ATLAS_GPU_CLEAR.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        let prev_active = (0x84C0 + CUR_UNIT.load(Ordering::Relaxed)) as i32;
        let mut prev_row_len = 0;
        (gl.get_integerv)(0x0CF2, &mut prev_row_len);
        (gl.pixel_storei)(0x0CF2, 0);
        (gl.active_texture)(GL_TEXTURE0 + atlas_unit());
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        // a new atlas texture, or a cleared atlas: made empty and cleared on the card, then only
        // the tiles with blood are sent (full upload only if the card can't clear it)
        let mut full_upload = false;
        if !a.allocated {
            (gl.tex_image_2d)(GL_TEXTURE_2D, 0, 0x822D /* R16F */, ATLAS_N as i32, ATLAS_N as i32, 0, 0x1903 /* RED */, 0x140B, std::ptr::null());
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            a.allocated = true;
            ATLAS_GPU_CLEAR.store(false, Ordering::Relaxed);
            full_upload = !gpu_clear_texture(gl, tex);
            (gl.bind_texture)(GL_TEXTURE_2D, tex);
        } else if ATLAS_GPU_CLEAR.swap(false, Ordering::Relaxed) {
            if !gpu_clear_texture(gl, tex) {
                a.dirty.mark_all();
            }
            (gl.bind_texture)(GL_TEXTURE_2D, tex);
        }
        if full_upload {
            let half: Vec<u16> = a.data.iter().map(|v| f16(*v)).collect();
            (gl.tex_image_2d)(GL_TEXTURE_2D, 0, 0x822D /* R16F */, ATLAS_N as i32, ATLAS_N as i32, 0, 0x1903 /* RED */, 0x140B, half.as_ptr() as *const c_void);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            a.allocated = true;
            a.dirty = Dirty::new(ATLAS_N, ATLAS_N);
        } else if a.dirty.any {
            let mut scratch: Vec<u16> = Vec::new();
            let t_up = std::time::Instant::now();
            let d = &mut a.dirty;
            'rows: for ty in 0..d.th {
                let mut tx = 0;
                while tx < d.tw {
                    if !d.bits[ty * d.tw + tx] {
                        tx += 1;
                        continue;
                    }
                    if t_up.elapsed().as_secs_f64() > UPLOAD_SECONDS {
                        break 'rows; // (the rest stays marked for the next frames)
                    }
                    let start = tx;
                    while tx < d.tw && d.bits[ty * d.tw + tx] {
                        d.bits[ty * d.tw + tx] = false;
                        tx += 1;
                    }
                    let (x0, y0) = (start * TILE, ty * TILE);
                    let (x1, y1) = ((tx * TILE).min(ATLAS_N), ((ty + 1) * TILE).min(ATLAS_N));
                    scratch.clear();
                    for y in y0..y1 {
                        scratch.extend(a.data[y * ATLAS_N + x0..y * ATLAS_N + x1].iter().map(|v| f16(*v)));
                    }
                    (gl.tex_sub_image_2d)(GL_TEXTURE_2D, 0, x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32, 0x1903, 0x140B, scratch.as_ptr() as *const c_void);
                }
            }
            d.any = d.bits.iter().any(|b| *b);
        }
        (gl.pixel_storei)(0x0CF2, prev_row_len);
        (gl.active_texture)(prev_active as u32);
    }
}

static PROP_HITS_LOGGED: AtomicU32 = AtomicU32::new(0);
fn stick_to_prop(p: PartRef, at: [f32; 3], n: [f32; 3], radius: f32) {
    if !RELEASE && PROP_HITS_LOGGED.fetch_add(1, Ordering::Relaxed) < 6 {
        let (_, _, _, t) = unsafe { part_pose(&p) };
        info_f!(
            "Cruor: drop hit object part 0x{:X} ({}) of object 0x{:X} ({}) at ({:.0}, {:.0}, {:.0}); part pose at ({:.0}, {:.0}, {:.0}), bounding radius {:.0}",
            p.part, obj_class_name(p.part), p.obj, obj_class_name(p.obj), at[0], at[1], at[2], t[0], t[1], t[2],
            unsafe { rf32(p.part + 0x2C) }
        );
    }
    let (r0, r1, r2, t) = unsafe { part_pose(&p) };
    let rel = sub3(at, t);
    let local = [dot3(r0, rel), dot3(r1, rel), dot3(r2, rel)];
    let ln = [dot3(r0, n), dot3(r1, n), dot3(r2, n)];
    let scale = dot3(r0, r0).sqrt().max(0.001);
    let frame = FRAME.load(Ordering::Relaxed);
    let (Ok(mut v), Ok(mut ag)) = (PROP_SPLATS.lock(), ATLAS.lock()) else { return };
    let a = ag.get_or_insert_with(|| ObjAtlas { data: vec![0.0; ATLAS_N * ATLAS_N], dirty: Dirty::new(ATLAS_N, ATLAS_N), allocated: false });
    let ob = match v.iter_mut().find(|(q, _)| q.part == p.part) {
        Some((_, ob)) => {
            ob.last = frame;
            *ob
        }
        None => {
            // a free slot, or the one bled on longest ago
            let mut used = vec![false; ATLAS_SLOTS];
            for (_, o) in v.iter() {
                used[o.slot] = true;
            }
            let slot = match used.iter().position(|u| !*u) {
                Some(sl) => sl,
                None => {
                    let k = v.iter().enumerate().min_by_key(|(_, (_, o))| o.last).map(|(k, _)| k).unwrap_or(0);
                    let sl = v[k].1.slot;
                    v.remove(k);
                    sl
                }
            };
            atlas_clear_slot(a, slot);
            // the box: the part's bounding sphere as the game stores it
            let (cw, rr) = unsafe { ({ let (o, c) = (rv3(p.obj + 0x64), rv3(p.part + 0x20)); [o[0] + c[0], o[1] + c[1], o[2] + c[2]] }, rf32(p.part + 0x2C)) };
            let crel = sub3(cw, t);
            let c = [dot3(r0, crel), dot3(r1, crel), dot3(r2, crel)];
            let r_local = ((if rr.is_finite() && rr > 0.0 { rr } else { 50.0 }) / scale * 1.1).max(5.0);
            let ob = ObjBlood { slot, c, inv_r: 1.0 / r_local, last: frame };
            v.push((p, ob));
            ob
        }
    };
    atlas_paint(a, &ob, local, ln, radius / scale);
    COL_PROP.fetch_add(1, Ordering::Relaxed);
}

/// Once per frame: where each bloodied prop is now, for matching the game's draws.
fn snapshot_prop_splats() {
    report_unmatched_props();
    let mut out = Vec::new();
    if let Ok(mut v) = PROP_SPLATS.lock() {
        v.retain(|(p, _)| unsafe { readable(p.part, 8) && (p.part as *const usize).read_unaligned() == p.vmt });
        for (p, ob) in v.iter() {
            let (r0, _, _, t) = unsafe { part_pose(p) };
            let (sx, sy) = slot_origin(ob.slot);
            let inv = 1.0 / ATLAS_N as f32;
            out.push((t, r0, vec![ob.c[0], ob.c[1], ob.c[2], ob.inv_r, sx as f32 * inv, sy as f32 * inv, OBJ_TILE as f32 * inv, OBJ_TILE as f32 * inv]));
        }
    }
    if let Ok(mut f) = PROP_FRAME.lock() {
        *f = out;
    }
}

// ---- Draw-call hooks: they tell us which 3D model each object is ----
static PENDING_OBJ: Mutex<Option<(u32, [f32; 16], bool)>> = Mutex::new(None);
static CUR_VAO: AtomicU32 = AtomicU32::new(0);
static REAL_BIND_VAO: AtomicUsize = AtomicUsize::new(0);
static REAL_DRAW_ELEMENTS: AtomicUsize = AtomicUsize::new(0);
static REAL_DRAW_RANGE: AtomicUsize = AtomicUsize::new(0);
type BindVaoFn = unsafe extern "system" fn(u32);
type DrawElementsFn = unsafe extern "system" fn(u32, i32, u32, *const c_void);
type DrawRangeFn = unsafe extern "system" fn(u32, u32, u32, i32, u32, *const c_void);

unsafe extern "system" fn my_bind_vertex_array(vao: u32) {
    CUR_VAO.store(vao, Ordering::Relaxed);
    let real: BindVaoFn = unsafe { std::mem::transmute(REAL_BIND_VAO.load(Ordering::Relaxed)) };
    unsafe { real(vao) }
}

fn before_draw(count: i32, offset: usize) {
    let prog = CURRENT_PROGRAM.load(Ordering::Relaxed);
    let pending = PENDING_OBJ.lock().ok().and_then(|g| *g);
    if let Some((p, m, movable)) = pending {
        if p == prog {
            object_stains_on_draw(prog, m, movable, (CUR_VAO.load(Ordering::Relaxed), count, offset));
        }
    }
}

unsafe extern "system" fn my_draw_elements(mode: u32, count: i32, ty: u32, indices: *const c_void) {
    DRAW_KINDS[0].fetch_add(1, Ordering::Relaxed);
    visit_with(|v| v.draws += 1);
    note_post_draw();
    before_draw(count, indices as usize);
    let real: DrawElementsFn = unsafe { std::mem::transmute(REAL_DRAW_ELEMENTS.load(Ordering::Relaxed)) };
    unsafe { real(mode, count, ty, indices) }
}

unsafe extern "system" fn my_draw_range_elements(mode: u32, start: u32, end: u32, count: i32, ty: u32, indices: *const c_void) {
    DRAW_KINDS[1].fetch_add(1, Ordering::Relaxed);
    visit_with(|v| v.draws += 1);
    note_post_draw();
    before_draw(count, indices as usize);
    let real: DrawRangeFn = unsafe { std::mem::transmute(REAL_DRAW_RANGE.load(Ordering::Relaxed)) };
    unsafe { real(mode, start, end, count, ty, indices) }
}

/// Is this draw placed exactly where a fixed level piece is? (within 1 cm)
fn is_static_piece(m: &[f32]) -> bool {
    let g = match LEVEL.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let idx = match g.as_ref() {
        Some(i) => i,
        None => return false,
    };
    let (x, y, z) = (m[12].round() as i32, m[13].round() as i32, m[14].round() as i32);
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                if idx.static_keys.contains(&(x + dx, y + dy, z + dz)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Level pieces are upright and square to the world; movable things aren't.
fn level_like(m: &[f32]) -> bool {
    let e = 0.001f32;
    let upright = m[1].abs() < e && m[4].abs() < e && m[6].abs() < e && m[9].abs() < e && m[5] > e;
    let square = (m[0].abs() < e || m[2].abs() < e) && (m[8].abs() < e || m[10].abs() < e);
    upright && square
}

/// Called for every object the game draws with its main shaders (right after it
/// uploads the object's placement): hand that object's splats to the shader.
/// The game draws each frame in several passes; a moving object is placed slightly
/// differently in each. Within 10 cm of where we already saw it this frame = same object.
fn same_object_this_frame(a: &[f32; 16], b: &[f32; 16]) -> bool {
    let dx = a[12] - b[12];
    let dy = a[13] - b[13];
    let dz = a[14] - b[14];
    dx * dx + dy * dy + dz * dz < 10.0 * 10.0
}

/// Per bloodied object this frame: the nearest draw seen (distance^2, alignment).
static PROP_NEAREST: Mutex<Vec<(f32, f32)>> = Mutex::new(Vec::new());
/// Per object (part address): frames in a row with no matching draw, and whether logged.
static PROP_UNMATCHED: Mutex<Option<std::collections::HashMap<usize, (u32, bool)>>> = Mutex::new(None);
fn report_unmatched_props() {
    if RELEASE {
        if let Ok(mut g) = PROP_NEAREST.lock() {
            g.clear();
        }
        return;
    }
    let nearest = match PROP_NEAREST.lock() {
        Ok(mut g) => std::mem::take(&mut *g),
        Err(_) => return,
    };
    let parts: Vec<(usize, usize)> = match PROP_SPLATS.lock() {
        Ok(v) => v.iter().map(|(p, _)| (p.part, p.obj)).collect(),
        Err(_) => return,
    };
    let Ok(mut g) = PROP_UNMATCHED.lock() else { return };
    let map = g.get_or_insert_with(Default::default);
    for (i, (part, obj)) in parts.iter().enumerate() {
        let (d2, al) = nearest.get(i).copied().unwrap_or((f32::INFINITY, 0.0));
        let matched = d2 < 100.0 && al > 0.97;
        let e = map.entry(*part).or_insert((0, false));
        if matched {
            e.0 = 0;
        } else {
            e.0 += 1;
            if e.0 > 120 && !e.1 {
                e.1 = true;
                warn_f!(
                    "Cruor: object part 0x{:X} ({}) of 0x{:X} ({}) has blood but no draw matched it - nearest draw {:.0} cm away, alignment {:.3} (needs < 10 cm and > 0.97)",
                    part, obj_class_name(*part), obj, obj_class_name(*obj),
                    if d2.is_finite() { d2.sqrt() } else { -1.0 }, al
                );
            }
        }
    }
}

fn object_stains_on_draw(program: u32, m: [f32; 16], movable: bool, key: (u32, i32, usize)) {
    let _ = (movable, key);
    // Which bloodied prop is this draw? Its placement matches the model's pose
    // (world offset within a few cm, same orientation). The game draws moving
    // objects in several passes per frame, slightly apart - hence the tolerance.
    let mut splats: Vec<f32> = Vec::new();
    if let Ok(f) = PROP_FRAME.lock() {
        let mut best: Option<(f32, usize)> = None;
        for (i, (t, r0, _)) in f.iter().enumerate() {
            let d2 = (t[0] - m[12]).powi(2) + (t[1] - m[13]).powi(2) + (t[2] - m[14]).powi(2);
            let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt().max(1e-6) * dot3(*r0, *r0).sqrt().max(1e-6);
            let align = (m[0] * r0[0] + m[1] * r0[1] + m[2] * r0[2]) / len;
            if d2 < 10.0 * 10.0 && align > 0.97 && best.map(|b| d2 < b.0).unwrap_or(true) {
                best = Some((d2, i));
            }
            if let Ok(mut nb) = PROP_NEAREST.try_lock() {
                if nb.len() < f.len() {
                    nb.resize(f.len(), (f32::INFINITY, 0.0));
                }
                if d2 < nb[i].0 {
                    nb[i] = (d2, align);
                }
            }
        }
        if let Some((_, i)) = best {
            splats = f[i].2.clone();
        }
    }
    // Upload to the shader.
    let glg = match STAIN_GL.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let gl = match glg.as_ref().and_then(|o| o.as_ref()) {
        Some(g) => g,
        None => return,
    };
    let locs = {
        let mut lg = match OBJ_LOCS.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let map = lg.get_or_insert_with(Default::default);
        *map.entry(program).or_insert_with(|| unsafe {
            (
                (gl.get_uniform_location)(program, b"PnObjBox\0".as_ptr() as *const c_char),
                (gl.get_uniform_location)(program, b"PnObjN\0".as_ptr() as *const c_char),
                (gl.get_uniform_location)(program, b"PnObjInv\0".as_ptr() as *const c_char),
                (gl.get_uniform_location)(program, b"PnObjTile\0".as_ptr() as *const c_char),
            )
        })
    };
    if locs.1 < 0 {
        return;
    }
    let on = BLOOD_ON.load(Ordering::Relaxed) && !UNIT_CONFLICT.load(Ordering::Relaxed);
    unsafe {
        if on && splats.len() >= 8 && locs.0 >= 0 && locs.2 >= 0 && locs.3 >= 0 {
            if let Some(inv) = mat_inverse(&m) {
                (gl.uniform_matrix4fv)(locs.2, 1, 0, inv.as_ptr());
                (gl.uniform4f)(locs.0, splats[0], splats[1], splats[2], splats[3]);
                (gl.uniform4f)(locs.3, splats[4], splats[5], splats[6], splats[7]);
                (gl.uniform1i)(locs.1, 1);
                return;
            }
        }
        (gl.uniform1i)(locs.1, 0);
    }
}

static SAMPLER_LOCS: Mutex<Option<std::collections::HashSet<(u32, i32)>>> = Mutex::new(None);

unsafe extern "system" fn my_uniform1i(loc: i32, v: i32) {
    if (0..128).contains(&v) {
        let prog = CURRENT_PROGRAM.load(Ordering::Relaxed);
        let is_sampler = SAMPLER_LOCS.lock().ok().map(|g| g.as_ref().map(|s| s.contains(&(prog, loc))).unwrap_or(false)).unwrap_or(false);
        if is_sampler {
            GAME_MAX_UNIT.fetch_max(v as u32, Ordering::Relaxed);
            let base = UNIT_BASE.load(Ordering::Relaxed) as i32;
            if base > 13 && v >= base && v <= base + 3 {
                GAME_USES_OUR_SLOT.store(v as u32, Ordering::Relaxed);
            }
        }
    }
    let real: Uniform1iFn = unsafe { std::mem::transmute(REAL_UNIFORM1I.load(Ordering::Relaxed)) };
    unsafe { real(loc, v) }
}
static REAL_ATTACH_SHADER: AtomicUsize = AtomicUsize::new(0);
static REAL_DELETE_PROGRAM: AtomicUsize = AtomicUsize::new(0);
static REAL_LINK_PROGRAM: AtomicUsize = AtomicUsize::new(0);
static REAL_DELETE_SHADER: AtomicUsize = AtomicUsize::new(0);
type ProgramFn = unsafe extern "system" fn(u32);

/// The game reuses program ID numbers: when one is deleted or (re)linked, forget
/// everything we remembered about it, so our settings never land in the wrong shader.
fn forget_program(program: u32, deleted: bool) {
    for m in [&STAIN_LOCS] {
        if let Ok(mut g) = m.lock() {
            if let Some(map) = g.as_mut() {
                map.remove(&program);
            }
        }
    }
    if let Ok(mut g) = OBJ_LOCS.lock() {
        if let Some(map) = g.as_mut() {
            map.remove(&program);
        }
    }
    if let Ok(mut g) = EXTRA_LOCS.lock() {
        if let Some(map) = g.as_mut() {
            map.remove(&program);
        }
    }
    if let Ok(mut g) = SAMPLER_LOCS.lock() {
        if let Some(set) = g.as_mut() {
            set.retain(|(p, _)| *p != program);
        }
    }
    if let Ok(mut g) = CAM.lock() {
        if let Some(c) = g.as_mut() {
            c.main_programs.remove(&program);
            c.objmat_locs.retain(|(p, _)| *p != program);
        }
    }
    if deleted {
        if let Ok(mut g) = MOVING_PROGRAMS.lock() {
            if let Some(set) = g.as_mut() {
                set.remove(&program);
            }
        }
    }
}

unsafe extern "system" fn my_delete_program(program: u32) {
    forget_program(program, true);
    let real: ProgramFn = unsafe { std::mem::transmute(REAL_DELETE_PROGRAM.load(Ordering::Relaxed)) };
    unsafe { real(program) }
}

unsafe extern "system" fn my_link_program(program: u32) {
    forget_program(program, false);
    let real: ProgramFn = unsafe { std::mem::transmute(REAL_LINK_PROGRAM.load(Ordering::Relaxed)) };
    unsafe { real(program) }
}

unsafe extern "system" fn my_delete_shader(shader: u32) {
    if let Ok(mut g) = MOVING_SHADERS.lock() {
        if let Some(set) = g.as_mut() {
            set.remove(&shader);
        }
    }
    if let Ok(mut g) = ORIGINAL_SOURCES.lock() {
        if let Some(map) = g.as_mut() {
            map.remove(&shader);
        }
    }
    let real: ProgramFn = unsafe { std::mem::transmute(REAL_DELETE_SHADER.load(Ordering::Relaxed)) };
    unsafe { real(shader) }
}
static MOVING_SHADERS: Mutex<Option<std::collections::HashSet<u32>>> = Mutex::new(None);
static MOVING_PROGRAMS: Mutex<Option<std::collections::HashSet<u32>>> = Mutex::new(None);
type AttachShaderFn = unsafe extern "system" fn(u32, u32);

unsafe extern "system" fn my_attach_shader(program: u32, shader: u32) {
    let moving = MOVING_SHADERS.lock().ok().map(|g| g.as_ref().map(|s| s.contains(&shader)).unwrap_or(false)).unwrap_or(false);
    if moving {
        if let Ok(mut g) = MOVING_PROGRAMS.lock() {
            let set = g.get_or_insert_with(Default::default);
            if set.insert(program) && set.len() % 20 == 1 {
                info_f!("Cruor: {} shader programs recognised as moving things (no stains on them)", set.len());
            }
        }
    }
    let real: AttachShaderFn = unsafe { std::mem::transmute(REAL_ATTACH_SHADER.load(Ordering::Relaxed)) };
    unsafe { real(program, shader) }
}

fn is_moving_program(program: u32) -> bool {
    MOVING_PROGRAMS.lock().ok().map(|g| g.as_ref().map(|s| s.contains(&program)).unwrap_or(false)).unwrap_or(false)
}
static REAL_COMPILE_SHADER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SOURCES: Mutex<Option<std::collections::HashMap<u32, Vec<u8>>>> = Mutex::new(None);
type ShaderSourceFn = unsafe extern "system" fn(u32, i32, *const *const c_char, *const i32);
type CompileShaderFn = unsafe extern "system" fn(u32);
type GetShaderivFn = unsafe extern "system" fn(u32, u32, *mut i32);

unsafe extern "system" fn my_shader_source(shader: u32, count: i32, strings: *const *const c_char, lengths: *const i32) {
    let real: ShaderSourceFn = unsafe { std::mem::transmute(REAL_SHADER_SOURCE.load(Ordering::Relaxed)) };
    // Gather the game's source text.
    let mut src: Vec<u8> = Vec::new();
    if !strings.is_null() {
        for i in 0..count.max(0) as usize {
            let p = unsafe { *strings.add(i) };
            if p.is_null() {
                continue;
            }
            let len = if lengths.is_null() { -1 } else { unsafe { *lengths.add(i) } };
            if len < 0 {
                src.extend_from_slice(unsafe { CStr::from_ptr(p) }.to_bytes());
            } else {
                src.extend_from_slice(unsafe { std::slice::from_raw_parts(p as *const u8, len as usize) });
            }
        }
    }
    let text = String::from_utf8_lossy(&src).into_owned();
    // Vertex shaders with per-vertex motion (vDlt) belong to moving things: characters, items.
    if text.contains("gl_Position") && text.contains("vDlt") {
        if let Ok(mut g) = MOVING_SHADERS.lock() {
            g.get_or_insert_with(Default::default).insert(shader);
        }
    }
    if wants_stains(&text) {
        let modified = format!("{}\n{}", text.replacen("void main()", "void pn_blood_game_main()", 1), STAIN_GLSL);
        if let Ok(c) = std::ffi::CString::new(modified) {
            if let Ok(mut g) = ORIGINAL_SOURCES.lock() {
                g.get_or_insert_with(Default::default).insert(shader, src.clone());
            }
            let ptr = c.as_ptr();
            unsafe { real(shader, 1, &ptr, std::ptr::null()) };
            return;
        }
    }
    unsafe { real(shader, count, strings, lengths) }
}

unsafe extern "system" fn my_compile_shader(shader: u32) {
    let real: CompileShaderFn = unsafe { std::mem::transmute(REAL_COMPILE_SHADER.load(Ordering::Relaxed)) };
    unsafe { real(shader) };
    let original = ORIGINAL_SOURCES.lock().ok().and_then(|mut g| g.as_mut().and_then(|m| m.remove(&shader)));
    if let Some(orig) = original {
        // We modified this one - make sure it still compiles; if not, put the game's version back.
        let getiv = unsafe { gl_resolve("glGetShaderiv") };
        if getiv.is_null() {
            return;
        }
        let getiv: GetShaderivFn = unsafe { std::mem::transmute(getiv) };
        let mut ok = 1;
        unsafe { getiv(shader, GL_COMPILE_STATUS, &mut ok) };
        if ok != 0 {
            let n = INJECTED_SHADERS.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 || n % 50 == 0 {
                info_f!("Cruor: in-game stains added to {} of the game's surface shaders", n);
            }
        } else {
            INJECT_FAILURES.fetch_add(1, Ordering::Relaxed);
            warn_f!("Cruor: a surface shader didn't accept the stain code; using the game's original for it");
            let ss: ShaderSourceFn = unsafe { std::mem::transmute(REAL_SHADER_SOURCE.load(Ordering::Relaxed)) };
            if let Ok(c) = std::ffi::CString::new(orig) {
                let ptr = c.as_ptr();
                unsafe {
                    ss(shader, 1, &ptr, std::ptr::null());
                    real(shader);
                }
            }
        }
    }
}

/// Uniform setters for the stain code, looked up straight from the driver.
struct StainGl {
    get_uniform_location: unsafe extern "system" fn(u32, *const c_char) -> i32,
    uniform1i: unsafe extern "system" fn(i32, i32),
    uniform1f: unsafe extern "system" fn(i32, f32),
    uniform4f: unsafe extern "system" fn(i32, f32, f32, f32, f32),
    uniform4fv: unsafe extern "system" fn(i32, i32, *const f32),
    uniform_matrix4fv: unsafe extern "system" fn(i32, i32, u8, *const f32),
}
static STAIN_GL: Mutex<Option<Option<StainGl>>> = Mutex::new(None);
static EXTRA_LOCS: Mutex<Option<std::collections::HashMap<u32, (i32, i32)>>> = Mutex::new(None);
static CURRENT_STAIN_ALLOWED: AtomicBool = AtomicBool::new(false);
static CURRENT_ON_LOC: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static STAIN_LOCS: Mutex<Option<std::collections::HashMap<u32, (i32, i32, i32, i32, i32, i32)>>> = Mutex::new(None);

/// Called right after the game switches to one of its shaders: feed it the blood map.
fn feed_stain_uniforms(program: u32) {
    if INJECTED_SHADERS.load(Ordering::Relaxed) == 0 || program == 0 {
        return;
    }
    let mut glg = match STAIN_GL.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if glg.is_none() {
        let load = || -> Option<StainGl> {
            unsafe {
                let a = gl_resolve("glGetUniformLocation");
                let b = gl_resolve("glUniform1i");
                let c = gl_resolve("glUniform1f");
                let d = gl_resolve("glUniform4f");
                let e = gl_resolve("glUniform4fv");
                let f = gl_resolve("glUniformMatrix4fv");
                if a.is_null() || b.is_null() || c.is_null() || d.is_null() || e.is_null() || f.is_null() {
                    return None;
                }
                Some(StainGl {
                    get_uniform_location: std::mem::transmute(a),
                    uniform1i: std::mem::transmute(b),
                    uniform1f: std::mem::transmute(c),
                    uniform4f: std::mem::transmute(d),
                    uniform4fv: std::mem::transmute(e),
                    uniform_matrix4fv: std::mem::transmute(f),
                })
            }
        };
        *glg = Some(load());
        // Pick our texture slots right away (before any stain code is used).
        unsafe {
            let gi = gl_resolve("glGetIntegerv");
            if !gi.is_null() {
                let gi: unsafe extern "system" fn(u32, *mut i32) = std::mem::transmute(gi);
                let mut max_units = 16;
                gi(0x8872, &mut max_units);
                let base = (max_units.max(16) - 4) as u32;
                if UNIT_BASE.swap(base, Ordering::Relaxed) != base {
                    info_f!("Cruor: graphics card has {} texture slots per shader; stains use slots {}-{}", max_units, base, base + 3);
                }
            }
        }
    }
    let gl = match glg.as_ref().and_then(|o| o.as_ref()) {
        Some(g) => g,
        None => return,
    };
    let locs = {
        let mut lg = match STAIN_LOCS.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let map = lg.get_or_insert_with(Default::default);
        *map.entry(program).or_insert_with(|| unsafe {
            (
                (gl.get_uniform_location)(program, b"PnBloodMap\0".as_ptr() as *const c_char),
                (gl.get_uniform_location)(program, b"PnBloodRect\0".as_ptr() as *const c_char),
                (gl.get_uniform_location)(program, b"PnBloodOn\0".as_ptr() as *const c_char),
                (gl.get_uniform_location)(program, b"PnWallX\0".as_ptr() as *const c_char),
                (gl.get_uniform_location)(program, b"PnWallZ\0".as_ptr() as *const c_char),
                (gl.get_uniform_location)(program, b"PnWallY\0".as_ptr() as *const c_char),
            )
        })
    };
    if locs.0 < 0 {
        return;
    }
    let rect = MAP_RECT.lock().map(|r| *r).unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let mode = STAIN_MODE.load(Ordering::Relaxed);
    let allowed = match mode {
        2 => !is_moving_program(program), // old rule: exclude shaders built for moving things
        _ => true,                        // level only (refined per object below) / everything
    };
    // Safety: if the game has ever assigned one of our texture slots, keep stains off.
    let conflict = GAME_USES_OUR_SLOT.load(Ordering::Relaxed) != 0;
    if conflict && !UNIT_CONFLICT.swap(true, Ordering::Relaxed) {
        warn_f!(
            "Cruor: the game uses texture slot {} which stains need - stains switched off to keep the game safe",
            GAME_USES_OUR_SLOT.load(Ordering::Relaxed)
        );
    }
    let on = if BLOOD_ON.load(Ordering::Relaxed) && !conflict && allowed { 1.0 } else { 0.0 };
    CURRENT_STAIN_ALLOWED.store(on > 0.5, Ordering::Relaxed);
    let dbg = if STAIN_DEBUG.load(Ordering::Relaxed) { 1.0 } else { 0.0 };
    unsafe {
        (gl.uniform1i)(locs.0, stain_unit() as i32);
        if locs.1 >= 0 {
            (gl.uniform4f)(locs.1, rect[0], rect[1], rect[2], rect[3]);
        }
        if locs.2 >= 0 {
            (gl.uniform1f)(locs.2, on);
        }
        let (dloc, kloc) = {
            let mut g = EXTRA_LOCS.lock().unwrap();
            *g.get_or_insert_with(Default::default).entry(program).or_insert_with(|| unsafe {
                (
                    (gl.get_uniform_location)(program, b"PnDebug\0".as_ptr() as *const c_char),
                    (gl.get_uniform_location)(program, b"PnDarkness\0".as_ptr() as *const c_char),
                )
            })
        };
        if dloc >= 0 {
            (gl.uniform1f)(dloc, dbg);
        }
        if kloc >= 0 {
            (gl.uniform1f)(kloc, tv(T_DARK));
        }
        {
            static ATLAS_LOCS: Mutex<Option<std::collections::HashMap<u32, i32>>> = Mutex::new(None);
            let aloc = ATLAS_LOCS.lock().ok().map(|mut g| {
                *g.get_or_insert_with(Default::default).entry(program).or_insert_with(|| unsafe {
                    (gl.get_uniform_location)(program, b"PnObjAtlas\0".as_ptr() as *const c_char)
                })
            });
            if let Some(l) = aloc {
                if l >= 0 {
                    (gl.uniform1i)(l, atlas_unit() as i32);
                }
            }
        }
        CURRENT_ON_LOC.store(locs.2, Ordering::Relaxed);
        if locs.3 >= 0 {
            (gl.uniform1i)(locs.3, wallx_unit() as i32);
        }
        if locs.4 >= 0 {
            (gl.uniform1i)(locs.4, wallz_unit() as i32);
        }
        if locs.5 >= 0 {
            let wy = WALL_Y.lock().map(|g| *g).unwrap_or([0.0, 0.0]);
            (gl.uniform4f)(locs.5, wy[0], wy[1], 0.0, 0.0);
        }
    }
}

/// Rebuild the blood map from the landed blood and upload it (render thread only).
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Paint one wall splat (round stain + drip) into a wall map.
fn raster_wall(
    data: &mut [f32], dirty: &mut Dirty, clip: [i64; 2], v0: f32, cu: f32, cy: f32, depth: f32, r: f32, seed: f32, streak: f32,
    shape: [f32; 4],
) {
    let (du, dv, stretch, spike) = (shape[0], shape[1], shape[2].max(1.0), shape[3]);
    // clip = global horizontal texel range (inclusive) allowed to be written
    let tu = MAP_SIZE / WALL_N as f32;
    let tv = WALL_HEIGHT / WALL_H as f32;
    // a crisp edge (under a texel of softening on small splats)
    let e = (0.8 * tu.max(tv)).max(r * 0.12);
    let reach = r * stretch * (1.0 + spike * 0.3) + e + 2.0 * tu.max(tv);
    let ph3 = ((seed * 20.0).cos(), (seed * 20.0).sin());
    let ph5 = ((seed * 13.0).cos(), (seed * 13.0).sin());
    let ph11 = ((seed * 41.0).cos(), (seed * 41.0).sin());
    let ph13 = ((seed * 53.0).cos(), (seed * 53.0).sin());
    let gu0 = (((cu - reach) / tu).floor() as i64).max(clip[0]);
    let gu1 = (((cu + reach) / tu).ceil() as i64).min(clip[1]);
    let iv0 = (((cy - streak - reach - v0) / tv).floor() as i64).max(0);
    let iv1 = (((cy + reach - v0) / tv).ceil() as i64).min(WALL_H as i64 - 1);
    if gu0 > gu1 || iv0 > iv1 {
        return;
    }
    let n = WALL_N as i64;
    for iv in iv0..=iv1 {
        for gu in gu0..=gu1 {
            let iu = gu.rem_euclid(n) as usize;
            let hu = (gu as f32 + 0.5) * tu - cu;
            let hz = hu.abs();
            let vv = v0 + (iv as f32 + 0.5) * tv - cy;
            // a round (or path-stretched, spiky) splat, or a streak (capsule) running
            // `streak` cm down from it
            let (dist, rim) = if streak <= 0.0 || vv > 0.0 {
                let a = hu * du + vv * dv;
                let bb = -hu * dv + vv * du;
                let sa = if a > 0.0 { stretch } else { 1.0 + (stretch - 1.0) * 0.2 };
                let au = a / sa;
                let taper = if a > 0.0 { 1.0 - 0.6 * (au / r.max(0.1)).clamp(0.0, 1.0) * (1.0 - 1.0 / stretch) } else { 1.0 };
                let thin = (stretch / 3.0).max(1.0).sqrt().min((r / (1.25 * tu.max(tv))).max(1.0));
                let bw = bb * thin / taper.max(0.2);
                let d0 = (au * au + bw * bw).sqrt();
                let front = if stretch > 1.3 { if a > 0.0 { 1.0 } else { 0.2 } } else { 1.0 };
                let z = if d0 > 1e-6 { (au / d0, bw / d0) } else { (1.0, 0.0) };
                let z2 = cmul(z, z);
                let z3 = cmul(z2, z);
                let z5 = cmul(z3, z2);
                let z11 = cmul(cmul(z5, z5), z);
                let z13 = cmul(z11, z2);
                (d0, r * (0.86 + (0.07 * wobble(z3, ph3) + 0.05 * wobble(z5, ph5)) / stretch
                    + spike * front * (0.14 * wobble(z11, ph11).max(0.0) + 0.10 * wobble(z13, ph13).max(0.0))))
            } else if vv < -streak {
                let dv = vv + streak;
                ((hz * hz + dv * dv).sqrt(), r * 0.9)
            } else {
                (hz, r * 0.9)
            };
            let amt = 1.0 - smooth((rim - e).max(0.0), rim + e * 0.5, dist);
            let k = (iv as usize * WALL_N + iu) * 2;
            if amt > data[k] {
                data[k] = amt;
                data[k + 1] = depth;
                dirty.mark(iu, iv as usize);
            } else if data[k] <= 0.0 && data[k + 1] != depth {
                data[k + 1] = depth;
                dirty.mark(iu, iv as usize);
            }
        }
    }
}

/// CPU copies of the three stain maps plus their placement; updated incrementally.
/// CRASH-PROOF LOG (test): important lines also go to blood-log.txt next to the settings,
/// written and flushed at once so they survive the game crashing.
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
fn file_log(line: &str) {
    use std::io::Write;
    if let Ok(mut g) = LOG_FILE.try_lock() {
        if g.is_none() {
            static TRIED: AtomicBool = AtomicBool::new(false);
            if TRIED.swap(true, Ordering::Relaxed) {
                return;
            }
            // Cruor-log.txt in the game folder (next to Exanima.exe); Documents\Cruor only if
            // Windows won't let us write there
            let game = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf()));
            let docs = std::env::var("USERPROFILE").ok().map(|h| std::path::PathBuf::from(h).join("Documents").join("Cruor"));
            for dir in [game, docs].into_iter().flatten() {
                let _ = std::fs::create_dir_all(&dir);
                let p = dir.join("Cruor-log.txt");
                if let Ok(f) = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&p) {
                    log::info!("Cruor: log file is {}", p.display());
                    *g = Some(f);
                    break;
                }
            }
        }
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{}", line);
            let _ = f.flush();
        }
    }
}

#[repr(C)]
struct ExceptionRecord {
    code: u32,
    flags: u32,
    record: *mut ExceptionRecord,
    address: *mut c_void,
    nparams: u32,
    info: [usize; 15],
}
#[repr(C)]
struct ExceptionPointers {
    record: *mut ExceptionRecord,
    context: *mut u8,
}
#[link(name = "kernel32")]
extern "system" {
    fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, addr: *const c_void, module: *mut usize) -> i32;
    fn GetModuleFileNameW(module: usize, buf: *mut u16, len: u32) -> u32;
}
/// "module+offset" for a code address.
fn where_is(addr: usize) -> String {
    let mut m = 0usize;
    let ok = unsafe { GetModuleHandleExW(0x4 | 0x2, addr as *const c_void, &mut m) };
    if ok == 0 || m == 0 {
        return format!("0x{:X} (no module)", addr);
    }
    let mut buf = [0u16; 260];
    let n = unsafe { GetModuleFileNameW(m, buf.as_mut_ptr(), 260) } as usize;
    let full = String::from_utf16_lossy(&buf[..n.min(260)]);
    let name = full.rsplit('\\').next().unwrap_or(&full).to_string();
    format!("{}+0x{:X}", name, addr - m)
}
static CRASHES_LOGGED: AtomicU32 = AtomicU32::new(0);
unsafe extern "system" fn crash_handler(ep: *mut ExceptionPointers) -> i32 {
    unsafe {
        if ep.is_null() || (*ep).record.is_null() {
            return 0;
        }
        let r = &*(*ep).record;
        let name = match r.code {
            0xC0000005 => "ACCESS VIOLATION",
            0xC00000FD => "STACK OVERFLOW",
            0xC000001D => "ILLEGAL INSTRUCTION",
            0xC0000094 => "INTEGER DIVIDE BY ZERO",
            0xC0000409 => "STACK BUFFER OVERRUN",
            _ => return 0, // (not a crash kind we report; let it go on as normal)
        };
        if CRASHES_LOGGED.fetch_add(1, Ordering::Relaxed) >= 8 {
            return 0;
        }
        let rip = r.address as usize;
        let detail = if r.code == 0xC0000005 && r.nparams >= 2 {
            format!("{} address 0x{:X}", match r.info[0] { 0 => "reading", 1 => "writing", 8 => "executing", _ => "accessing" }, r.info[1])
        } else {
            String::new()
        };
        let st = STAGE.load(Ordering::Relaxed);
        let tid = GetCurrentThreadId();
        file_log(&format!(
            "Cruor CRASH RECORD: {} {} at {} - thread {} (blood's thread {}), plugin stage {} ({})",
            name, detail, where_is(rip), tid, FRAME_THREAD.load(Ordering::Relaxed), st, stage_name(st)
        ));
        // return addresses on the stack that point into the plugin or the game
        let ctx = (*ep).context;
        if !ctx.is_null() {
            let rsp = (ctx.add(0x98) as *const usize).read_unaligned();
            let mut found = Vec::new();
            for i in 0..256usize {
                let a = rsp + i * 8;
                // (straight to Windows: the crash may have happened inside the cached check)
                let mut mi: MemInfo = std::mem::zeroed();
                if VirtualQuery(a as *const c_void, &mut mi, std::mem::size_of::<MemInfo>()) == 0
                    || mi.state != 0x1000
                    || a + 8 > mi.base + mi.size
                {
                    break;
                }
                let v = (a as *const usize).read_unaligned();
                if is_code(v) {
                    found.push(where_is(v));
                    if found.len() >= 12 {
                        break;
                    }
                }
            }
            file_log(&format!("Cruor CRASH RECORD: stack: {}", found.join(" <- ")));
        }
        0 // EXCEPTION_CONTINUE_SEARCH: the game's own handling goes on as usual
    }
}
fn install_crash_handler() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::Relaxed) {
        unsafe { AddVectoredExceptionHandler(1, crash_handler) };
        file_log("Cruor: crash recorder installed (blood-log.txt)");
    }
}

/// PERFORMANCE (test): per-frame figures, reported every 5 s.
static PERF_HOOK_NS: AtomicU64 = AtomicU64::new(0);
static PERF_HOOK_CALLS: AtomicU64 = AtomicU64::new(0);
static PERF_INSCENE_NS: AtomicU64 = AtomicU64::new(0);
static PERF_PREP_NS: AtomicU64 = AtomicU64::new(0);
static PERF_FRAME: Mutex<(Option<std::time::Instant>, f64, f64, u32)> = Mutex::new((None, 0.0, 0.0, 0));
fn perf_add(c: &AtomicU64, since: std::time::Instant) {
    c.fetch_add(since.elapsed().as_nanos() as u64, Ordering::Relaxed);
}
/// Called at the start of every frame: frame-to-frame time.
fn perf_frame_tick() {
    // the GPU timers rotate through 4 query slots, one per frame (results read 4 frames later)
    if let Ok(mut q) = GPU_Q.lock() {
        q.2 = q.2.wrapping_add(1);
    }
    if let Ok(mut f) = PERF_FRAME.lock() {
        let now = std::time::Instant::now();
        if let Some(t) = f.0 {
            let ms = (now - t).as_secs_f64() * 1000.0;
            f.1 += ms;
            f.2 = f.2.max(ms);
            f.3 += 1;
        }
        f.0 = Some(now);
    }
}
/// GPU time of the plugin's in-scene passes (0 = depth copy / window draw, 1 = combined
/// draw), read a few frames later so nothing waits for the graphics card.
static GPU_Q: Mutex<([[u32; 4]; 2], [[bool; 4]; 2], u32, [f64; 2], [u32; 2])> = Mutex::new(([[0; 4]; 2], [[false; 4]; 2], 0, [0.0; 2], [0; 2]));
static GPU_ACTIVE: AtomicBool = AtomicBool::new(false);
fn gpu_timer_begin(which: usize) {
    if RELEASE || GPU_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let Ok(cell) = RENDERER.lock() else { return };
    let Some(r) = cell.0.as_ref() else { return };
    let gl = &r.gl;
    if let Ok(mut q) = GPU_Q.lock() {
        let slot = (q.2 % 4) as usize;
        unsafe {
            if q.0[which][slot] == 0 {
                (gl.gen_queries)(4, q.0[which].as_mut_ptr());
            }
            // collect the result this slot held (4 frames ago) if it's ready
            if q.1[which][slot] {
                let mut avail = 0u32;
                (gl.get_query_objectuiv)(q.0[which][slot], 0x8867 /* RESULT_AVAILABLE */, &mut avail);
                if avail != 0 {
                    let mut ns = 0u32;
                    (gl.get_query_objectuiv)(q.0[which][slot], 0x8866 /* RESULT */, &mut ns);
                    q.3[which] += ns as f64 / 1.0e6;
                    q.4[which] += 1;
                }
                q.1[which][slot] = false;
            }
            (gl.begin_query)(0x88BF /* TIME_ELAPSED */, q.0[which][slot]);
            q.1[which][slot] = true;
        }
        GPU_ACTIVE.store(true, Ordering::Relaxed);
    }
}
fn gpu_timer_end() {
    if !GPU_ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    let Ok(cell) = RENDERER.lock() else { return };
    let Some(r) = cell.0.as_ref() else { return };
    unsafe { (r.gl.end_query)(0x88BF) };
}
fn perf_report() {
    if RELEASE {
        // (keep the counters from growing; nothing is logged in the release build)
        if let Ok(mut f) = PERF_FRAME.lock() {
            f.1 = 0.0;
            f.2 = 0.0;
            f.3 = 0;
        }
        for c in DRAW_KINDS.iter() {
            c.store(0, Ordering::Relaxed);
        }
        PERF_HOOK_NS.store(0, Ordering::Relaxed);
        PERF_HOOK_CALLS.store(0, Ordering::Relaxed);
        PERF_INSCENE_NS.store(0, Ordering::Relaxed);
        PERF_PREP_NS.store(0, Ordering::Relaxed);
        return;
    }
    let (frames, avg, worst) = match PERF_FRAME.lock() {
        Ok(mut f) => {
            let r = (f.3, f.1 / f.3.max(1) as f64, f.2);
            f.1 = 0.0;
            f.2 = 0.0;
            f.3 = 0;
            r
        }
        Err(_) => return,
    };
    let per = |c: &AtomicU64| c.swap(0, Ordering::Relaxed) as f64 / 1.0e6 / frames.max(1) as f64;
    let hook = per(&PERF_HOOK_NS);
    let calls = PERF_HOOK_CALLS.swap(0, Ordering::Relaxed) / frames.max(1) as u64;
    let inscene = per(&PERF_INSCENE_NS);
    let prep = per(&PERF_PREP_NS);
    let gpu = match GPU_Q.lock() {
        Ok(mut q) => {
            let g = [q.3[0] / q.4[0].max(1) as f64, q.3[1] / q.4[1].max(1) as f64];
            q.3 = [0.0; 2];
            q.4 = [0; 2];
            g
        }
        Err(_) => [0.0; 2],
    };
    {
        let k: Vec<u32> = DRAW_KINDS.iter().map(|c| c.swap(0, Ordering::Relaxed) / frames.max(1)).collect();
        info_f!(
            "Cruor PERF (5 s): game draw calls per frame - elements {}, range {}, instanced {}, arrays {}, arrays-instanced {}, multi-elements {}, multi-arrays {}",
            k[0], k[1], k[2], k[3], k[4], k[5], k[6]
        );
    }
    info_f!(
        "Cruor PERF (5 s): frame avg {:.2} ms ({:.0} fps), worst {:.1} ms | plugin CPU per frame: simulation+collision+maps {:.2} ms, camera hook {:.3} ms ({} calls), flying-blood passes {:.2} ms | GPU: depth copy/window draw {:.2} ms, combined draw {:.2} ms",
        avg, 1000.0 / avg.max(0.001), worst, prep, hook, calls, inscene, gpu[0], gpu[1]
    );
}

/// HANG WATCHDOG (test): which part of the plugin the game thread is in right now.
static STAGE: AtomicU32 = AtomicU32::new(0);
fn stage_name(s: u32) -> &'static str {
    match s {
        0 => "NOT in the plugin (the game itself is busy or stuck)",
        1 => "in the plugin: drop simulation",
        2 => "in the plugin: drop collision",
        3 => "in the plugin: wall runners",
        4 => "in the plugin: pooling",
        5 => "in the plugin: stain maps (window / strips)",
        6 => "in the plugin: stain maps (painting)",
        7 => "in the plugin: stain maps (upload)",
        8 => "in the plugin: drawing flying blood",
        9 => "in the plugin: body list",
        10 => "in the plugin: level collision index",
        11 => "in the plugin: level collision index (reading a slice)",
        12 => "in the plugin: level collision index (starting a read)",
        13 => "in the plugin: level collision index (adding objects the game added)",
        14 => "in the plugin: level collision index (refreshing the prop grid)",
        15 => "in the plugin: level collision index (waiting for the collision lock)",
        16 => "in GAME code called by the plugin: the game's ray test (drop collision)",
        17 => "in GAME code called by the plugin: the game's add-blood-entry (drop hit a character)",
        _ => "in the plugin (unknown part)",
    }
}
fn set_stage(s: u32) {
    STAGE.store(s, Ordering::Relaxed);
}
/// Started once: notices when no new frame has begun for 3 s and logs where the game
/// thread is (once per hang, again if it lasts 10 s).
fn start_watchdog() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        let mut last = FRAME.load(Ordering::Relaxed);
        let mut stuck = 0.0f32;
        let mut reported = 0;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let f = FRAME.load(Ordering::Relaxed);
            if f != last {
                if reported > 0 {
                    warn_f!("Cruor WATCHDOG: frames resumed after {:.1} s", stuck);
                }
                last = f;
                stuck = 0.0;
                reported = 0;
                continue;
            }
            stuck += 0.5;
            if f > 300 && ((stuck >= 3.0 && reported == 0) || (stuck >= 10.0 && reported == 1)) {
                reported += 1;
                let st = STAGE.load(Ordering::Relaxed);
                let (blobs, walls, runners) = match SIM.try_lock() {
                    Ok(sim) => (sim.blobs.len() as i64, sim.walls.len() as i64, sim.runners.len() as i64),
                    Err(std::sync::TryLockError::Poisoned(_)) => (-2, -2, -2),
                    Err(std::sync::TryLockError::WouldBlock) => (-1, -1, -1),
                };
                warn_f!(
"Cruor WATCHDOG: no new frame for {:.0} s - the game thread is {} (stage {}); drops {}, wall splats {}, runners {} (-1 = lock busy, -2 = lock POISONED by a panic); presents from other threads so far: {}",
                    stuck, stage_name(st), st, blobs, walls, runners, OTHER_THREAD_PRESENTS.load(Ordering::Relaxed)
                );
            }
        }
    });
}

/// 32-bit float -> 16-bit float (round to nearest), for uploads.
fn f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xFF) as i32;
    let man = b & 0x7F_FFFF;
    if exp == 255 {
        return sign | 0x7C00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = (man | 0x80_0000) >> (1 - e);
        return sign | ((m + 0x1000) >> 13) as u16;
    }
    let v = ((e as u32) << 10) | (man >> 13);
    let round = (man >> 12) & 1;
    sign | (v + round) as u16
}

/// Splat outline wobble without atan2/sin per texel: with (c, s) the direction to the
/// texel (cos, sin of its angle), sin(n*angle + phase) = Im((c + i s)^n * e^(i phase)).
#[inline]
fn cmul(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}
#[inline]
fn wobble(zn: (f32, f32), ph: (f32, f32)) -> f32 {
    // sin(n*a + p) with zn = (cos na, sin na), ph = (cos p, sin p)
    zn.1 * ph.0 + zn.0 * ph.1
}

/// Changed 32x32 tiles of a map, uploaded one row-run at a time (at most UPLOAD_BUDGET
/// bytes a frame; the rest goes next frame).
const TILE: usize = 32;
const UPLOAD_BUDGET: u64 = 4 << 20;
/// ...and at most this much time per frame (a burst of landings is sent over a few frames
/// instead of stalling one).
const UPLOAD_SECONDS: f64 = 0.00075;
/// Newly landed splats painted per frame at most (the rest are painted the next frames).
const NEW_FLOOR_PER_FRAME: usize = 150;
const NEW_WALL_PER_FRAME: usize = 100;
/// The stain textures exist at their full size (re-made only with a new renderer): a new
/// set of maps (clear, level change) only clears them on the card.
static STAIN_TEX_READY: AtomicBool = AtomicBool::new(false);
struct Dirty {
    tw: usize,
    th: usize,
    bits: Vec<bool>,
    any: bool,
}
impl Dirty {
    fn new(w: usize, h: usize) -> Dirty {
        let (tw, th) = ((w + TILE - 1) / TILE, (h + TILE - 1) / TILE);
        Dirty { tw, th, bits: vec![false; tw * th], any: false }
    }
    fn mark(&mut self, x: usize, y: usize) {
        let k = (y / TILE) * self.tw + x / TILE;
        if !self.bits[k] {
            self.bits[k] = true;
            self.any = true;
        }
    }
    fn mark_all(&mut self) {
        self.bits.iter_mut().for_each(|b| *b = true);
        self.any = true;
    }
}

/// WRAP-AROUND MAPS: a texel's storage place is its WORLD texel number wrapped around the
/// map size (floor: x and z; walls: the horizontal axis). The window - the 24 m around
/// the player that is valid - moves in 25 cm steps; moving only clears and refills the
/// strip that comes into range. Nothing is rebuilt when the player walks around.
const WINDOW_STEP: f32 = 18.75; // cm: exactly 16 floor texels, 24 wall texels
struct Maps {
    floor: Vec<f32>, // MAP_N x MAP_N x 4: amount, height, height tolerance, unused
    wx: Vec<f32>,    // WALL_N x WALL_H x 2 (walls facing along z; horizontal = world x)
    wz: Vec<f32>,    // (walls facing along x; horizontal = world z)
    /// Window origin (world cm, multiple of WINDOW_STEP) and the wall maps' base height.
    x0: f32,
    z0: f32,
    y0: f32,
    allocated: bool,
    df: Dirty,
    dwx: Dirty,
    dwz: Dirty,
}
static MAPS: Mutex<Option<Maps>> = Mutex::new(None);

/// Paint one floor stain into the floor map; returns the rows touched.
fn raster_floor(
    data: &mut [f32], dirty: &mut Dirty, clip: [i64; 4], px: f32, py: f32, pz: f32, r: f32, tol: f32, shape: [f32; 4],
) {
    let (sdx, sdz, stretch, spike) = (shape[0], shape[1], shape[2].max(1.0), shape[3]);
    // nothing smaller than the map can draw (a speck is at least ~2 texels across)
    let r = r.max(0.9 * MAP_SIZE / MAP_N as f32);
    // clip = global texel ranges [x0, x1, z0, z1] (inclusive) allowed to be written
    let texel = MAP_SIZE / MAP_N as f32;
    let seed = ((px * 12.9898 + pz * 78.233).sin() * 43758.547).fract().abs();
    let (p1, p2, p3) = ((seed * 6.283).cos(), (seed * 6.283).sin(), (seed * 17.0).cos());
    let (p1, p2, p3) = ((p1, p2), (p3, (seed * 17.0).sin()), ((seed * 29.0).cos(), (seed * 29.0).sin()));
    let p4 = ((seed * 41.0).cos(), (seed * 41.0).sin());
    let p5 = ((seed * 53.0).cos(), (seed * 53.0).sin());
    let e_max = (r * 0.3).max(1.5 * texel);
    let reach = r * 1.2 * stretch * (1.0 + spike * 0.3) + e_max + 2.0 * texel;
    let gx0 = (((px - reach) / texel).floor() as i64).max(clip[0]);
    let gx1 = (((px + reach) / texel).ceil() as i64).min(clip[1]);
    let gz0 = (((pz - reach) / texel).floor() as i64).max(clip[2]);
    let gz1 = (((pz + reach) / texel).ceil() as i64).min(clip[3]);
    if gx0 > gx1 || gz0 > gz1 {
        return;
    }
    let n = MAP_N as i64;
    for gz in gz0..=gz1 {
        let iz = gz.rem_euclid(n) as usize;
        for gx in gx0..=gx1 {
            let ix = gx.rem_euclid(n) as usize;
            let wx = (gx as f32 + 0.5) * texel - px;
            let wz = (gz as f32 + 0.5) * texel - pz;
            // into the splat's own frame: along its path (a) and across it (b); the
            // leading half is stretched more than the trailing half (a teardrop)
            let a = wx * sdx + wz * sdz;
            let bb = -wx * sdz + wz * sdx;
            // a streak: long and tapering to a point ahead (the way it travelled),
            // round behind
            let sa = if a > 0.0 { stretch } else { 1.0 + (stretch - 1.0) * 0.2 };
            let au = a / sa;
            let taper = if a > 0.0 { 1.0 - 0.6 * (au / r.max(0.1)).clamp(0.0, 1.0) * (1.0 - 1.0 / stretch) } else { 1.0 };
            // long streaks get thinner (needle lines) - but never thinner than the map
            // can draw, or they smear into faint scratches
            let thin = (stretch / 3.0).max(1.0).sqrt().min((r / (1.25 * texel)).max(1.0));
            let bw = bb * thin / taper.max(0.2);
            let dist = (au * au + bw * bw).sqrt();
            // a streak's fingers only on its leading edge
            let front = if stretch > 1.3 { if a > 0.0 { 1.0 } else { 0.2 } } else { 1.0 };
            let z = if dist > 1e-6 { (au / dist, bw / dist) } else { (1.0, 0.0) };
            let z2 = cmul(z, z);
            let z4 = cmul(z2, z2);
            let z5 = cmul(z4, z);
            let z9 = cmul(cmul(z4, z4), z);
            let z11 = cmul(z9, z2);
            let z15 = cmul(cmul(z9, z4), z2);
            let z17 = cmul(z15, z2);
            // ragged edge for round splats, smoother the longer the streak
            let rough = 1.0 / stretch;
            let rim = 0.8 + rough * (0.1 * wobble(z5, p1) + 0.06 * wobble(z9, p2) + 0.04 * wobble(z15, p3))
                + spike * front * (0.14 * wobble(z11, p4).max(0.0) + 0.10 * wobble(z17, p5).max(0.0));
            let edge = r * rim;
            // a crisp edge (under a texel of softening on small splats)
            let e = (edge * 0.12).max(0.7 * texel);
            let k = (iz * MAP_N + ix) * 4;
            let amt = if dist >= edge + 0.5 * e { 0.0 } else { 1.0 - smooth((edge - e).max(0.0), edge + 0.5 * e, dist) };
            if amt > data[k] {
                data[k] = amt;
                data[k + 1] = py;
                data[k + 2] = tol;
                dirty.mark(ix, iz);
            } else if data[k] <= 0.0 && (data[k + 1] != py || data[k + 2] != tol) {
                // empty texel beside a stain: give it the stain's height so smooth
                // filtering at the edge only fades the amount
                data[k + 1] = py;
                data[k + 2] = tol;
                dirty.mark(ix, iz);
            }
        }
    }
}

/// Height (and tolerance) to paint a landed stain at.
fn stain_height(b: &Blob) -> (f32, f32) {
    if b.exact {
        return (b.pos[1], if b.band > 0.0 { b.band } else { 16.0 });
    }
    let (bias, learned) = floor_bias();
    let h = local_floor(b.pos[0], b.pos[2]).unwrap_or(if learned { b.floor_y + bias } else { b.pos[1] });
    (h, 60.0) // not measured yet: a wider band, still well below knee height
}

fn merge_rows(a: Option<(usize, usize)>, b: Option<(usize, usize)>) -> Option<(usize, usize)> {
    match (a, b) {
        (Some(x), Some(y)) => Some((x.0.min(y.0), x.1.max(y.1))),
        (x, None) => x,
        (None, y) => y,
    }
}

/// TEST: where the plugin's frame time goes (5-second totals): 0 stain maps (paint +
/// upload), 1 (bytes uploaded), 2 body-list rebuild, 3 runners, 4 pooling.
static COSTS: Mutex<[(f64, f64, u64, u32); 9]> = Mutex::new([(0.0, 0.0, 0, 0); 9]);
fn cost_note(kind: usize, ms: f64, bytes: u64) {
    if let Ok(mut c) = COSTS.lock() {
        let e = &mut c[kind];
        e.0 += ms;
        e.1 = e.1.max(ms);
        e.2 += bytes;
        e.3 += 1;
    }
}
fn cost_report() {
    if RELEASE {
        if let Ok(mut c) = COSTS.lock() {
            *c = [(0.0, 0.0, 0, 0); 9];
        }
        let _ = SIM_TIME.lock().map(|mut t| *t = (0.0, 0.0, 0));
        let _ = VERT_TIME.lock().map(|mut t| *t = (0.0, 0.0, 0));
        VQ_CALLS.store(0, Ordering::Relaxed);
        VQ_NS.store(0, Ordering::Relaxed);
        return;
    }
    let take = |m: &Mutex<(f64, f64, u32)>| m.lock().map(|mut st| { let r = *st; *st = (0.0, 0.0, 0); r }).unwrap_or((0.0, 0.0, 0));
    let (ss, sw, sn) = take(&SIM_TIME);
    let (vs, vw, vn) = take(&VERT_TIME);
    if let Ok(mut c) = COSTS.lock() {
        let f = |e: &(f64, f64, u64, u32)| format!("avg {:.2} ms, worst {:.2} ms", e.0 / e.3.max(1) as f64, e.1);
        info_f!(
            "Cruor PERF (5 s): WHOLE blood step avg {:.2} ms, worst {:.2} ms | in it: cast-off {}; wound drips {}; fluid forces + movement {}; collision {} (of which body list {}); wall runners {}; pooling {} | building the drawing avg {:.2} ms, worst {:.2} ms | stain maps {} ({:.1} MB uploaded) | memory checks (VirtualQuery) {:.0} per frame, {:.3} ms per frame",
            ss / sn.max(1) as f64, sw, f(&c[7]), f(&c[8]), f(&c[5]), f(&c[6]), f(&c[2]), f(&c[3]), f(&c[4]), vs / vn.max(1) as f64, vw, f(&c[0]), c[1].2 as f64 / 1.0e6,
            VQ_CALLS.swap(0, Ordering::Relaxed) as f64 / sn.max(1) as f64, VQ_NS.swap(0, Ordering::Relaxed) as f64 / 1.0e6 / sn.max(1) as f64
        );
        *c = [(0.0, 0.0, 0, 0); 9];
    }
}

/// Clear a texture to zero on the graphics card (attached to a framebuffer of the plugin's own,
/// every bit of GL state it touches put back). false = couldn't (the caller uploads instead).
unsafe fn gpu_clear_texture(gl: &Gl, tex: u32) -> bool {
    static FBO: AtomicU32 = AtomicU32::new(0);
    unsafe {
        let mut fbo = FBO.load(Ordering::Relaxed);
        if fbo == 0 {
            (gl.gen_framebuffers)(1, &mut fbo);
            FBO.store(fbo, Ordering::Relaxed);
        }
        if fbo == 0 {
            return false;
        }
        let mut prev_draw = 0i32;
        (gl.get_integerv)(0x8CA6 /* DRAW_FRAMEBUFFER_BINDING */, &mut prev_draw);
        let scissor_on = (gl.is_enabled)(0x0C11 /* SCISSOR_TEST */) != 0;
        let mut cm = [0u8; 4];
        (gl.get_booleanv)(0x0C23 /* COLOR_WRITEMASK */, cm.as_mut_ptr());
        let mut cc = [0.0f32; 4];
        (gl.get_floatv)(0x0C22 /* COLOR_CLEAR_VALUE */, cc.as_mut_ptr());
        (gl.bind_framebuffer)(0x8CA9 /* DRAW_FRAMEBUFFER */, fbo);
        (gl.framebuffer_texture_2d)(0x8CA9, 0x8CE0 /* COLOR_ATTACHMENT0 */, GL_TEXTURE_2D, tex, 0);
        (gl.draw_buffer)(0x8CE0);
        let ok = (gl.check_framebuffer_status)(0x8CA9) == 0x8CD5 /* COMPLETE */;
        if ok {
            (gl.disable)(0x0C11);
            (gl.color_mask)(1, 1, 1, 1);
            (gl.clear_color)(0.0, 0.0, 0.0, 0.0);
            (gl.clear)(0x4000 /* COLOR_BUFFER_BIT */);
        }
        (gl.framebuffer_texture_2d)(0x8CA9, 0x8CE0, GL_TEXTURE_2D, 0, 0);
        (gl.bind_framebuffer)(0x8CA9, prev_draw as u32);
        if scissor_on {
            (gl.enable)(0x0C11);
        }
        (gl.color_mask)(cm[0], cm[1], cm[2], cm[3]);
        (gl.clear_color)(cc[0], cc[1], cc[2], cc[3]);
        if !GPU_CLEAR_LOGGED.swap(true, Ordering::Relaxed) {
            test_f!("Cruor PERF: stain maps are cleared on the graphics card ({})", if ok { "working" } else { "NOT available - full uploads as before" });
        }
        ok
    }
}
static GPU_CLEAR_LOGGED: AtomicBool = AtomicBool::new(false);

unsafe fn update_blood_map(gl: &Gl, tex: u32, wallx_tex: u32, wallz_tex: u32) {
    let t_map = std::time::Instant::now();
    set_stage(5);
    unsafe { update_blood_map_inner(gl, tex, wallx_tex, wallz_tex) };
    set_stage(0);
    cost_note(0, t_map.elapsed().as_secs_f64() * 1000.0, 0);
}

unsafe fn update_blood_map_inner(gl: &Gl, tex: u32, wallx_tex: u32, wallz_tex: u32) {
    let mut mg = match MAPS.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let ftex = MAP_SIZE / MAP_N as f32;
    let wtex = MAP_SIZE / WALL_N as f32;
    let full = MAP_DIRTY.load(Ordering::Relaxed) || mg.is_none();
    if full {
        // full rebuilds only for clearing / first use / level change (at most 5 a second)
        if let Ok(mut last) = LAST_MAP_BUILD.lock() {
            if let Some(t) = *last {
                if t.elapsed().as_secs_f32() < 0.2 && mg.is_some() {
                    return;
                }
            }
            *last = Some(std::time::Instant::now());
        }
        MAP_DIRTY.store(false, Ordering::Relaxed);
    }
    // where the window should be (follows the player in 25 cm steps)
    let hips = player_hips().or_else(|| {
        let f = LAST_SPRAY_POS.lock().ok().and_then(|p| *p)?;
        if let Some(m) = mg.as_ref() {
            let (cx, cz) = (m.x0 + MAP_SIZE * 0.5, m.z0 + MAP_SIZE * 0.5);
            if (f[0] - cx).abs() < MAP_SIZE * 0.3 && (f[2] - cz).abs() < MAP_SIZE * 0.3 {
                return None; // still well inside: keep the window where it is
            }
        }
        Some(f)
    });
    let (want_x0, want_z0) = match hips {
        Some(h) => (
            ((h[0] - MAP_SIZE * 0.5) / WINDOW_STEP).floor() * WINDOW_STEP,
            ((h[2] - MAP_SIZE * 0.5) / WINDOW_STEP).floor() * WINDOW_STEP,
        ),
        None => match mg.as_ref() {
            Some(m) => (m.x0, m.z0),
            None => (-MAP_SIZE * 0.5, -MAP_SIZE * 0.5),
        },
    };
    let want_y0 = {
        let mut g = WALL_Y0.lock().unwrap();
        if let Some(h) = hips {
            let outside = match *g {
                Some(y0) => h[1] < y0 + 150.0 || h[1] > y0 + WALL_HEIGHT - 300.0,
                None => true,
            };
            if outside {
                *g = Some(h[1] - 300.0);
            }
        }
        (*g).unwrap_or(0.0)
    };
    if full {
        *mg = Some(Maps {
            floor: vec![0.0; MAP_N * MAP_N * 4],
            wx: vec![0.0; WALL_N * WALL_H * 2],
            wz: vec![0.0; WALL_N * WALL_H * 2],
            x0: want_x0,
            z0: want_z0,
            y0: want_y0,
            allocated: false,
            df: Dirty::new(MAP_N, MAP_N),
            dwx: Dirty::new(WALL_N, WALL_H),
            dwz: Dirty::new(WALL_N, WALL_H),
        });
    }
    let maps = mg.as_mut().unwrap();
    let gf = |x0: f32| (x0 / ftex).round() as i64; // window start, floor texels
    let gw = |x0: f32| (x0 / wtex).round() as i64; // window start, wall texels
    let nf = MAP_N as i64;
    let nw = WALL_N as i64;

    // ---- what changed: strips that came into range, the walls' height band, new stains ----
    // floor strips (x then z), wall strips (x map along x, z map along z)
    let mut floor_strips: Vec<[i64; 4]> = Vec::new();
    let mut wx_strips: Vec<[i64; 2]> = Vec::new();
    let mut wz_strips: Vec<[i64; 2]> = Vec::new();
    let mut refill_all_floor = full;
    let mut refill_all_walls = full;
    if !full {
        let (ox, oz) = (maps.x0, maps.z0);
        let dxs = gf(want_x0) - gf(ox);
        let dzs = gf(want_z0) - gf(oz);
        if dxs.abs() >= nf || dzs.abs() >= nf {
            refill_all_floor = true;
            refill_all_walls = true;
        } else {
            // clear the strips that come into range and list them for refilling
            let clear_floor_cols = |maps: &mut Maps, c0: i64, c1: i64| {
                for gx in c0..=c1 {
                    let ix = gx.rem_euclid(nf) as usize;
                    for iz in 0..MAP_N {
                        let k = (iz * MAP_N + ix) * 4;
                        if maps.floor[k] != 0.0 || maps.floor[k + 1] != 0.0 || maps.floor[k + 2] != 0.0 {
                            maps.floor[k] = 0.0;
                            maps.floor[k + 1] = 0.0;
                            maps.floor[k + 2] = 0.0;
                            maps.df.mark(ix, iz);
                        }
                    }
                }
            };
            let clear_floor_rows = |maps: &mut Maps, r0: i64, r1: i64| {
                for gz in r0..=r1 {
                    let iz = gz.rem_euclid(nf) as usize;
                    for ix in 0..MAP_N {
                        let k = (iz * MAP_N + ix) * 4;
                        if maps.floor[k] != 0.0 || maps.floor[k + 1] != 0.0 || maps.floor[k + 2] != 0.0 {
                            maps.floor[k] = 0.0;
                            maps.floor[k + 1] = 0.0;
                            maps.floor[k + 2] = 0.0;
                            maps.df.mark(ix, iz);
                        }
                    }
                }
            };
            let clear_wall_cols = |data: &mut Vec<f32>, dirty: &mut Dirty, c0: i64, c1: i64| {
                for gu in c0..=c1 {
                    let iu = gu.rem_euclid(nw) as usize;
                    for iv in 0..WALL_H {
                        let k = (iv * WALL_N + iu) * 2;
                        if data[k] != 0.0 || data[k + 1] != 0.0 {
                            data[k] = 0.0;
                            data[k + 1] = 0.0;
                            dirty.mark(iu, iv);
                        }
                    }
                }
            };
            let entering = |old: i64, new: i64, n: i64| -> Option<(i64, i64)> {
                if new > old {
                    Some((old + n, new + n - 1))
                } else if new < old {
                    Some((new, old - 1))
                } else {
                    None
                }
            };
            let (fx_old, fx_new, fz_old, fz_new) = (gf(ox), gf(want_x0), gf(oz), gf(want_z0));
            if let Some((c0, c1)) = entering(fx_old, fx_new, nf) {
                clear_floor_cols(maps, c0, c1);
                floor_strips.push([c0, c1, fz_old, fz_old + nf - 1]);
            }
            if let Some((r0, r1)) = entering(fz_old, fz_new, nf) {
                clear_floor_rows(maps, r0, r1);
                floor_strips.push([fx_new, fx_new + nf - 1, r0, r1]);
            }
            if let Some((c0, c1)) = entering(gw(ox), gw(want_x0), nw) {
                clear_wall_cols(&mut maps.wx, &mut maps.dwx, c0, c1);
                wx_strips.push([c0, c1]);
            }
            if let Some((c0, c1)) = entering(gw(oz), gw(want_z0), nw) {
                clear_wall_cols(&mut maps.wz, &mut maps.dwz, c0, c1);
                wz_strips.push([c0, c1]);
            }
        }
        if (want_y0 - maps.y0).abs() > 0.5 {
            refill_all_walls = true; // changed floors (vertically): the wall maps start over
        }
        if refill_all_floor {
            maps.floor.iter_mut().for_each(|v| *v = 0.0);
            maps.df.mark_all();
        }
        if refill_all_walls {
            maps.wx.iter_mut().for_each(|v| *v = 0.0);
            maps.wz.iter_mut().for_each(|v| *v = 0.0);
            maps.dwx.mark_all();
            maps.dwz.mark_all();
        }
        maps.x0 = want_x0;
        maps.z0 = want_z0;
        maps.y0 = want_y0;
    }
    let (x0, z0, y0) = (maps.x0, maps.z0, maps.y0);
    let fwin = [gf(x0), gf(x0) + nf - 1, gf(z0), gf(z0) + nf - 1];
    let wxwin = [gw(x0), gw(x0) + nw - 1];
    let wzwin = [gw(z0), gw(z0) + nw - 1];
    let need_all = refill_all_floor || refill_all_walls || !floor_strips.is_empty() || !wx_strips.is_empty() || !wz_strips.is_empty();
    // stains to paint: new ones always; all of them when strips/refills need them
    let (puddles, walls): (Vec<(f32, f32, f32, f32, f32, bool, [f32; 4])>, Vec<(WallSplat, bool)>) = match SIM.lock() {
        Ok(mut sim) => {
            let mut p = Vec::new();
            let mut new_f = 0usize;
            for b in sim.blobs.iter_mut() {
                if b.landed && b.radius > 0.01 {
                    let new = !b.mapped;
                    if new && new_f >= NEW_FLOOR_PER_FRAME {
                        continue; // painted next frame
                    }
                    if new {
                        new_f += 1;
                    }
                    if new || need_all {
                        let (h, tol) = stain_height(b);
                        p.push((b.pos[0], h, b.pos[2], b.radius, tol, new, b.shape));
                    }
                    b.mapped = true;
                }
            }
            let mut w = Vec::new();
            let mut new_w = 0usize;
            for ws in sim.walls.iter_mut() {
                let new = !ws.mapped;
                if new && new_w >= NEW_WALL_PER_FRAME {
                    continue; // painted next frame
                }
                if new {
                    new_w += 1;
                }
                if new || need_all {
                    w.push((*ws, new));
                }
                ws.mapped = true;
            }
            (p, w)
        }
        Err(_) => return,
    };
    set_stage(6);
    // the runners' moving tips (painted only)
    if let Ok(mut tips) = RUNNER_TIPS.lock() {
        for w in tips.drain(..) {
            if w.normal[2].abs() >= w.normal[0].abs() {
                raster_wall(&mut maps.wx, &mut maps.dwx, wxwin, y0, w.pos[0], w.pos[1], w.pos[2], w.radius, w.seed, w.streak, w.shape);
            } else {
                raster_wall(&mut maps.wz, &mut maps.dwz, wzwin, y0, w.pos[2], w.pos[1], w.pos[0], w.radius, w.seed, w.streak, w.shape);
            }
        }
    }
    // floor
    for &(px, py, pz, r, tol, new, shape) in puddles.iter() {
        if new || refill_all_floor {
            raster_floor(&mut maps.floor, &mut maps.df, fwin, px, py, pz, r, tol, shape);
        } else {
            for st in floor_strips.iter() {
                let clip = [st[0].max(fwin[0]), st[1].min(fwin[1]), st[2].max(fwin[2]), st[3].min(fwin[3])];
                raster_floor(&mut maps.floor, &mut maps.df, clip, px, py, pz, r, tol, shape);
            }
        }
    }
    // walls
    for (w, new) in walls.iter() {
        let along_x = w.normal[2].abs() >= w.normal[0].abs();
        let (data, dirty, win, strips, cu, depth) = if along_x {
            (&mut maps.wx, &mut maps.dwx, wxwin, &wx_strips, w.pos[0], w.pos[2])
        } else {
            (&mut maps.wz, &mut maps.dwz, wzwin, &wz_strips, w.pos[2], w.pos[0])
        };
        if *new || refill_all_walls {
            raster_wall(data, dirty, win, y0, cu, w.pos[1], depth, w.radius, w.seed, w.streak, w.shape);
        } else {
            for st in strips.iter() {
                let clip = [st[0].max(win[0]), st[1].min(win[1])];
                raster_wall(data, dirty, clip, y0, cu, w.pos[1], depth, w.radius, w.seed, w.streak, w.shape);
            }
        }
    }
    // ---- upload: whole textures the first time, afterwards only changed tiles ----
    set_stage(7);
    let mut uploaded: u64 = 0;
    // nothing changed: nothing to send (and no driver queries) this frame
    let need_upload = !maps.allocated || maps.df.any || maps.dwx.any || maps.dwz.any;
    if need_upload { unsafe {
        let prev_active = (0x84C0 + CUR_UNIT.load(Ordering::Relaxed)) as i32;
        let mut prev_row_len = 0;
        (gl.get_integerv)(0x0CF2 /* UNPACK_ROW_LENGTH */, &mut prev_row_len);
        let set_params = |gl: &Gl, wrap_t: i32| unsafe {
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, 0x2901 /* REPEAT */);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, wrap_t);
        };
        // upload changed tiles, a run of neighbouring tiles in a row at a time, as 16-bit
        // values (the textures are 16-bit: converting here halves the data and spares the
        // driver its own conversion)
        let mut scratch: Vec<u16> = Vec::new();
        let mut budget: u64 = UPLOAD_BUDGET;
        let t_up = std::time::Instant::now();
        let mut upload_tiles = |gl: &Gl, d: &mut Dirty, data: &[f32], w: usize, h: usize, ch: usize, fmt: u32| -> u64 {
            let mut bytes = 0u64;
            if !d.any || budget == 0 {
                return 0;
            }
            unsafe { (gl.pixel_storei)(0x0CF2, 0) };
            for ty in 0..d.th {
                let mut tx = 0;
                while tx < d.tw {
                    if !d.bits[ty * d.tw + tx] {
                        tx += 1;
                        continue;
                    }
                    if bytes >= budget || t_up.elapsed().as_secs_f64() > UPLOAD_SECONDS {
                        break; // (over this frame's budget: the rest stays marked for next frame)
                    }
                    let start = tx;
                    while tx < d.tw && d.bits[ty * d.tw + tx] {
                        d.bits[ty * d.tw + tx] = false;
                        tx += 1;
                    }
                    let (x0p, y0p) = (start * TILE, ty * TILE);
                    let (x1p, y1p) = ((tx * TILE).min(w), ((ty + 1) * TILE).min(h));
                    scratch.clear();
                    for y in y0p..y1p {
                        let row = &data[(y * w + x0p) * ch..(y * w + x1p) * ch];
                        scratch.extend(row.iter().map(|v| f16(*v)));
                    }
                    unsafe {
                        (gl.tex_sub_image_2d)(GL_TEXTURE_2D, 0, x0p as i32, y0p as i32, (x1p - x0p) as i32, (y1p - y0p) as i32, fmt, 0x140B /* HALF_FLOAT */, scratch.as_ptr() as *const c_void)
                    };
                    bytes += ((x1p - x0p) * (y1p - y0p) * ch * 2) as u64;
                }
            }
            d.any = d.bits.iter().any(|b| *b);
            budget = budget.saturating_sub(bytes);
            bytes
        };
        (gl.active_texture)(GL_TEXTURE0 + stain_unit());
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        if !maps.allocated {
            // new maps start blank: make the texture empty and clear it on the graphics card,
            // then send only the tiles that have blood (instead of 33 MB of zeros at once)
            (gl.pixel_storei)(0x0CF2, 0);
            if !STAIN_TEX_READY.load(Ordering::Relaxed) {
                (gl.tex_image_2d)(GL_TEXTURE_2D, 0, 0x881A /* RGBA16F */, MAP_N as i32, MAP_N as i32, 0, GL_RGBA, 0x140B, std::ptr::null());
                set_params(gl, 0x2901);
            }
            if gpu_clear_texture(gl, tex) {
                uploaded += upload_tiles(gl, &mut maps.df, &maps.floor, MAP_N, MAP_N, 4, GL_RGBA);
            } else {
                let half: Vec<u16> = maps.floor.iter().map(|v| f16(*v)).collect();
                (gl.tex_image_2d)(GL_TEXTURE_2D, 0, 0x881A, MAP_N as i32, MAP_N as i32, 0, GL_RGBA, 0x140B, half.as_ptr() as *const c_void);
                uploaded += (MAP_N * MAP_N * 8) as u64;
                maps.df = Dirty::new(MAP_N, MAP_N);
            }
            (gl.bind_texture)(GL_TEXTURE_2D, tex);
        } else {
            uploaded += upload_tiles(gl, &mut maps.df, &maps.floor, MAP_N, MAP_N, 4, GL_RGBA);
        }
        for (unit, t, which) in [(wallx_unit(), wallx_tex, 0), (wallz_unit(), wallz_tex, 1)] {
            (gl.active_texture)(GL_TEXTURE0 + unit);
            (gl.bind_texture)(GL_TEXTURE_2D, t);
            let (data, dirty) = if which == 0 { (&maps.wx, &mut maps.dwx) } else { (&maps.wz, &mut maps.dwz) };
            if !maps.allocated {
                (gl.pixel_storei)(0x0CF2, 0);
                if !STAIN_TEX_READY.load(Ordering::Relaxed) {
                    (gl.tex_image_2d)(GL_TEXTURE_2D, 0, GL_RG16F, WALL_N as i32, WALL_H as i32, 0, GL_RG, 0x140B, std::ptr::null());
                    set_params(gl, GL_CLAMP_TO_EDGE);
                }
                if gpu_clear_texture(gl, t) {
                    uploaded += upload_tiles(gl, dirty, data, WALL_N, WALL_H, 2, GL_RG);
                } else {
                    let half: Vec<u16> = data.iter().map(|v| f16(*v)).collect();
                    (gl.tex_image_2d)(GL_TEXTURE_2D, 0, GL_RG16F, WALL_N as i32, WALL_H as i32, 0, GL_RG, 0x140B, half.as_ptr() as *const c_void);
                    uploaded += (WALL_N * WALL_H * 4) as u64;
                    *dirty = Dirty::new(WALL_N, WALL_H);
                }
                (gl.bind_texture)(GL_TEXTURE_2D, t);
            } else {
                uploaded += upload_tiles(gl, dirty, data, WALL_N, WALL_H, 2, GL_RG);
            }
        }
        (gl.pixel_storei)(0x0CF2, prev_row_len);
        (gl.active_texture)(prev_active as u32);
        cost_note(1, 0.0, uploaded);
        STAIN_TEX_READY.store(true, Ordering::Relaxed);
    } }
    maps.allocated = true;
    if let Ok(mut r) = MAP_RECT.lock() {
        *r = [x0, z0, 1.0 / MAP_SIZE, 1.0 / MAP_SIZE];
    }
    if let Ok(mut g) = WALL_Y.lock() {
        *g = [y0, 1.0 / WALL_HEIGHT];
    }
}

// ============================================================
// Per-frame driver (runs in the game's present function)
// ============================================================
static FRAME: AtomicU32 = AtomicU32::new(0);

/// Remove all blood (level changed, restarted, loading screen, or F8).
fn clear_blood(reason: &str) {
    MAP_DIRTY.store(true, Ordering::Relaxed);
    // (the level's collision is NOT re-read here: that happens only when the level's
    //  objects actually change - see update_level_index - so a camera jump can't hitch)
    if let Ok(mut v) = PROP_SPLATS.lock() {
        v.clear();
    }
    atlas_clear_all();
    if let Ok(mut sim) = SIM.lock() {
        if sim.blobs.is_empty() && sim.drips.is_empty() {
            return;
        }
        sim.blobs.clear();
        sim.drips.clear();
        sim.walls.clear();
        sim.runners.clear();
        if let Ok(mut t) = TRACKS.lock() {
            t.clear();
        }
    }
    if reason != "F8" {
        if let Ok(mut g) = FLOOR_BIAS.lock() {
            *g = (0.0, 0);
        }
    }
    ui_say(&format!("Cruor: blood cleared ({})", reason), UI_WHITE);
}
static KEYS_DOWN: AtomicU32 = AtomicU32::new(0);
static VP_HISTORY: Mutex<Vec<Mat4>> = Mutex::new(Vec::new());

fn poll_hotkeys() {
    drip_test_poll();
    // F5..F9 = virtual keys 0x74..0x78
    let names = ["blood", "camera delay", "hide behind things", "clear", "fluid behaviour"];
    let prev = KEYS_DOWN.load(Ordering::Relaxed);
    let mut now = 0u32;
    for k in 0..5u32 {
        if k != 0 && k != 3 {
            continue; // release build: only F5 (blood on/off) and F8 (clear)
        }
        let down = unsafe { GetAsyncKeyState(0x74 + k as i32) } as u16 & 0x8000 != 0;
        if down {
            now |= 1 << k;
        }
        if down && prev & (1 << k) == 0 {
            let flag = match k {
                0 => Some(&BLOOD_ON),
                1 => Some(&DELAY_ON),
                2 => Some(&DEPTH_ON),
                4 => Some(&FLUID_ON),
                _ => None,
            };
            match flag {
                Some(f) => {
                    let v = !f.load(Ordering::Relaxed);
                    f.store(v, Ordering::Relaxed);
                    ui_say(&format!("Cruor: {} {}", names[k as usize], if v { "ON" } else { "OFF" }), UI_WHITE);
                }
                None => clear_blood("F8"),
            }
        }
    }
    KEYS_DOWN.store(now, Ordering::Relaxed);
}

// ---- Test hotkey: hold F10 to spew blood in front of your character ----
static SPEW_NEXT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// Where the game keeps its pointer to the player's character. Found at startup by
/// reading it out of the damage function's code ("is this the player?" check).
static PLAYER_SLOT: AtomicUsize = AtomicUsize::new(0);
/// CHARACTER LAYOUT: the 0.9.5.2 public version and the newer beta lay the physics body
/// and character out differently. Detected at startup from the game's own code (the field
/// offset inside the player-slot instruction): 0xE68 = public layout, 0xD58 = beta layout.
/// Offsets that moved (public -> beta): body-part corners +0xB0 -> +0xF0, blood entries
/// count/array +0x89C/+0x8A0 -> +0x78C/+0x790, worn-items model +0x12F0 -> +0x11E0, sector
/// +0x5F0 -> +0x4D0, position +0x360 -> +0x240. Unknown layout: the character features stay off.
static OFF_CORNERS: AtomicUsize = AtomicUsize::new(0xB0);
static OFF_BLOOD_N: AtomicUsize = AtomicUsize::new(0x89C);
static OFF_BLOOD_ARR: AtomicUsize = AtomicUsize::new(0x8A0);
static OFF_ITEMS: AtomicUsize = AtomicUsize::new(0x12F0);
static OFF_SECTOR: AtomicUsize = AtomicUsize::new(0x5F0);
static LAYOUT_OK: AtomicBool = AtomicBool::new(false);
fn corners() -> usize { OFF_CORNERS.load(Ordering::Relaxed) }
fn blood_n() -> usize { OFF_BLOOD_N.load(Ordering::Relaxed) }
fn blood_arr() -> usize { OFF_BLOOD_ARR.load(Ordering::Relaxed) }
fn items_off() -> usize { OFF_ITEMS.load(Ordering::Relaxed) }
fn sector_off() -> usize { OFF_SECTOR.load(Ordering::Relaxed) }
const PLAYER_SLOT_SIG: &str = "89 83 ?? ?? 00 00 90 48 3B 1D ?? ?? ?? ?? 75 13 48 83 3D";

/// SPECTATING (no player character): the game spraying blood is proof its level is in use, so
/// a spray switches the level collision on even if no player camera ever does its collision
/// test. The last bleeder stands in for the player when finding characters, and the last
/// spray position for where the stain maps should be. Forgotten at every level unload.
static LAST_BLEEDER: AtomicUsize = AtomicUsize::new(0);
static LAST_SPRAY_POS: Mutex<Option<[f32; 3]>> = Mutex::new(None);
static SPECTATE_LOGGED: AtomicBool = AtomicBool::new(false);
pub(crate) fn note_spray(bleeder: usize, at: [f32; 3]) {
    if bleeder != 0 {
        LAST_BLEEDER.store(bleeder, Ordering::Relaxed);
    }
    if let Ok(mut p) = LAST_SPRAY_POS.lock() {
        *p = Some(at);
    }
    if SCENE.load(Ordering::Relaxed) != 0 && !WORLD_LIVE.swap(true, Ordering::Relaxed) && player_hips().is_none() && !SPECTATE_LOGGED.swap(true, Ordering::Relaxed) {
        info_f!("Cruor: no player character in this fight (spectating) - level collision started from the game's own blood spray; stains follow the fight");
    }
}
fn forget_spectate() {
    LAST_BLEEDER.store(0, Ordering::Relaxed);
    if let Ok(mut p) = LAST_SPRAY_POS.lock() {
        *p = None;
    }
}

fn player_hips() -> Option<[f32; 3]> {
    let slot = PLAYER_SLOT.load(Ordering::Relaxed);
    if slot == 0 {
        return None;
    }
    let player = unsafe { (slot as *const usize).read_unaligned() };
    if player < 0x10000 || player % 8 != 0 {
        return None;
    }
    let hip = unsafe { read_hip(player) };
    if hip.iter().all(|v| v.is_finite() && v.abs() < 1.0e6) { Some(hip) } else { None }
}

// ---- Numpad tuning: 8/2 pick a setting, 4/6 or -/+ change it, 5 reset it ----
static TUNE_KEYS: Mutex<[(bool, Option<std::time::Instant>, Option<std::time::Instant>); 7]> =
    Mutex::new([(false, None, None); 7]);

fn settings_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("mods").join("Cruor").join("Cruor-settings.txt"))
}

fn save_settings() {
    let vals = match TUNE_VALUES.lock() {
        Ok(v) => *v,
        Err(_) => return,
    };
    let mut text = String::from("# Cruor settings (change in game with the numpad; this file is saved automatically)\n");
    for (i, t) in TUNE.iter().enumerate() {
        text.push_str(&format!("{} = {:.3}\n", t.0, vals[i]));
    }
    if let Some(p) = settings_path() {
        // (the first save, and any failure, go to the log and on screen - a save must never
        // fail silently)
        static SAVED_ONCE: AtomicBool = AtomicBool::new(false);
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&p, text) {
            Ok(()) => {
                if !SAVED_ONCE.swap(true, Ordering::Relaxed) {
                    info_f!("Cruor: settings saved to {}", p.display());
                }
            }
            Err(e) => {
                warn_f!("Cruor: couldn't save the settings to {} ({})", p.display(), e);
                ui_say(&format!("Cruor: couldn't save settings ({})", e), UI_WHITE);
            }
        }
    }
}

fn load_settings() {
    let p = match settings_path() {
        Some(p) => p,
        None => return,
    };
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => return,
    };
    if let Ok(mut v) = TUNE_VALUES.lock() {
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("");
            if let Some((k, val)) = line.split_once('=') {
                if let (Some(i), Ok(x)) = (TUNE.iter().position(|t| t.0 == k.trim()), val.trim().parse::<f32>()) {
                    v[i] = x.clamp(TUNE[i].2, TUNE[i].3);
                }
            }
        }
    }
    info_f!("Cruor: settings loaded from {}", p.display());
}

fn show_setting(i: usize) {
    let v = tv(i);
    ui_say(
        &format!("Cruor {} = {:.2}   [{}/{}]  Numpad 8/2 pick, 4/6 change, 5 reset", TUNE[i].0, v, i + 1, TUNE.len()),
        UI_WHITE,
    );
}

fn tune_keys_poll() {
    // Numpad 8, 2, 4, 6, 5, -, +
    const KEYS: [i32; 7] = [0x68, 0x62, 0x64, 0x66, 0x65, 0x6D, 0x6B];
    let now = std::time::Instant::now();
    let mut actions: Vec<usize> = Vec::new();
    if let Ok(mut st) = TUNE_KEYS.lock() {
        for (k, vk) in KEYS.iter().enumerate() {
            let down = unsafe { GetAsyncKeyState(*vk) } as u16 & 0x8000 != 0;
            let entry = &mut st[k];
            if down && !entry.0 {
                actions.push(k);
                entry.1 = Some(now);
                entry.2 = Some(now);
            } else if down && (k == 2 || k == 3 || k == 5 || k == 6) {
                // holding change keys repeats
                if let (Some(start), Some(last)) = (entry.1, entry.2) {
                    if (now - start).as_millis() > 350 && (now - last).as_millis() > 90 {
                        actions.push(k);
                        entry.2 = Some(now);
                    }
                }
            }
            entry.0 = down;
        }
    }
    if actions.is_empty() {
        return;
    }
    let n = TUNE.len() as u32;
    for a in actions {
        let sel = TUNE_SELECTED.load(Ordering::Relaxed) as usize;
        match a {
            0 => TUNE_SELECTED.store((sel as u32 + 1) % n, Ordering::Relaxed),
            1 => TUNE_SELECTED.store((sel as u32 + n - 1) % n, Ordering::Relaxed),
            _ => {
                let (_, def, lo, hi, step) = TUNE[sel];
                if let Ok(mut v) = TUNE_VALUES.lock() {
                    let up = a == 3 || a == 6;
                    v[sel] = match a {
                        4 => def,
                        _ if step < 0.0 => (v[sel] + if up { -step } else { step }).clamp(lo, hi),
                        _ => {
                            let x = if up { v[sel].max(0.05) * step } else { v[sel] / step };
                            (if x < 0.03 { 0.0 } else { x }).clamp(lo, hi)
                        }
                    };
                }
                save_settings();
            }
        }
        show_setting(TUNE_SELECTED.load(Ordering::Relaxed) as usize);
    }
}


/// Is [addr, addr+len) committed, readable memory?
/// Readable memory regions already reported by Windows, by 64 KB block: (start, end).
/// Forgotten when the level unloads and every 10 s, so freed memory isn't trusted.
/// Memory regions checked this frame: region start -> end, exactly as VirtualQuery reports them,
/// so any address inside an already-checked region is answered without asking Windows again
/// (each VirtualQuery costs ~50 us here). Still forgotten every frame.
static REGIONS: Mutex<Option<std::collections::BTreeMap<usize, usize>>> = Mutex::new(None);
fn forget_regions() {
    if let Ok(mut g) = REGIONS.lock() {
        *g = None;
    }
}
/// TEST (performance pass): VirtualQuery calls made by readable(), and their total time.
static VQ_CALLS: AtomicU64 = AtomicU64::new(0);
static VQ_NS: AtomicU64 = AtomicU64::new(0);
fn readable(addr: usize, len: usize) -> bool {
    if addr < 0x10000 {
        return false;
    }
    let end = addr.saturating_add(len);
    if let Ok(g) = REGIONS.lock() {
        if let Some(m) = g.as_ref() {
            if let Some((&b, &e)) = m.range(..=addr).next_back() {
                if addr >= b && end <= e {
                    return true;
                }
            }
        }
    }
    let mut mi: MemInfo = unsafe { std::mem::zeroed() };
    let tq = std::time::Instant::now();
    let n = unsafe { VirtualQuery(addr as *const c_void, &mut mi, std::mem::size_of::<MemInfo>()) };
    VQ_CALLS.fetch_add(1, Ordering::Relaxed);
    VQ_NS.fetch_add(tq.elapsed().as_nanos() as u64, Ordering::Relaxed);
    if n == 0 || mi.state != 0x1000 {
        return false;
    }
    let bad = mi.protect & 0x01 != 0 || mi.protect & 0x100 != 0; // NOACCESS / GUARD
    if bad {
        return false;
    }
    let (b, e) = (mi.base, mi.base + mi.size);
    if let Ok(mut g) = REGIONS.lock() {
        let m = g.get_or_insert_with(Default::default);
        if m.len() > 200_000 {
            m.clear();
        }
        m.insert(b, e);
    }
    end <= e
}
unsafe fn rd_usize(a: usize) -> Option<usize> {
    if readable(a, 8) { Some(unsafe { (a as *const usize).read_unaligned() }) } else { None }
}
unsafe fn rd_i32(a: usize) -> Option<i32> {
    if readable(a, 4) { Some(unsafe { (a as *const i32).read_unaligned() }) } else { None }
}

/// Class name of a game object (reads its class record; image-range checked).
fn obj_class_name(obj: usize) -> String {
    unsafe {
        let base = GetModuleHandleA(std::ptr::null()) as usize;
        let e_lfanew = ((base + 0x3C) as *const u32).read_unaligned() as usize;
        let size = ((base + e_lfanew + 0x18 + 0x38) as *const u32).read_unaligned() as usize;
        let inside = |p: usize| p >= base && p < base + size;
        let vmt = match rd_usize(obj) { Some(v) if inside(v) => v, _ => return "?".into() };
        let np = match rd_usize(vmt + 0x18) { Some(v) if inside(v) => v, _ => return "?".into() };
        let len = *(np as *const u8) as usize;
        String::from_utf8_lossy(std::slice::from_raw_parts((np + 1) as *const u8, len.min(64))).into_owned()
    }
}

fn spew_poll() {
    let down = unsafe { GetAsyncKeyState(0x79) } as u16 & 0x8000 != 0; // F10
    if !down {
        return;
    }
    let now = std::time::Instant::now();
    if let Ok(mut next) = SPEW_NEXT.lock() {
        if let Some(t) = *next {
            if now < t {
                return;
            }
        }
        *next = Some(now + std::time::Duration::from_millis(120));
    }
    let hip = match player_hips() {
        Some(h) => h,
        None => {
            warn_f!("Cruor: F10 spew - couldn't find your character");
            return;
        }
    };
    let fwd = current_view_proj().map(|m| norm3([m[3], 0.0, m[11]])).unwrap_or([1.0, 0.0, 0.0]);
    let from = [hip[0] + fwd[0] * 35.0, hip[1] + 30.0, hip[2] + fwd[2] * 35.0];
    let dir = [fwd[0], 0.55, fwd[2]];
    spawn_jet(from, dir, 30.0, hip[1] - HIP_HEIGHT, 1.0);
}

fn prepare_frame() {
    install_panic_log();
    install_crash_handler();
    forget_regions(); // remembered memory regions only ever trusted within one frame
    perf_frame_tick();
    let t_prep = std::time::Instant::now();
    // a panic in the plugin must never unwind into the game: log it, reset the locks
    // it left broken, and carry on
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(prepare_frame_inner)).is_err() {
        set_stage(0);
        SIM.clear_poison();
        LEVEL.clear_poison();
        LEVEL_BUILD.clear_poison();
        MAPS.clear_poison();
    }
    perf_add(&PERF_PREP_NS, t_prep);
}

/// Every panic in the plugin is logged with where it happened (once each place).
fn install_panic_log() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_else(|| "?".into());
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "(no message)".into()
        };
        let first = SEEN.try_lock().map(|mut v| if v.contains(&at) { false } else { v.push(at.clone()); true }).unwrap_or(true);
        if first {
            warn_f!("Cruor PANIC at {} (stage {}): {}", at, STAGE.load(Ordering::Relaxed), msg);
        }
        let _ = &default;
    }));
}

fn prepare_frame_inner() {
    let frame = FRAME.fetch_add(1, Ordering::Relaxed);
    gl_verify_tick();

    if let (Ok(mut f), Ok(mut l)) = (FRAME_OBJECTS.lock(), LAST_OBJECTS.lock()) {
        std::mem::swap(&mut *f, &mut *l);
        f.clear();
    }
    start_watchdog();
    poll_hotkeys();
    set_stage(10);
    update_level_index();
    set_stage(0);
    if PROBE_REQ.swap(false, Ordering::Relaxed) {
        stairs_probe();
        character_model_probe();
        BOM_LOGGED.store(0, Ordering::Relaxed); // log the game's next blood-on-mesh calls again
    }
    snapshot_prop_splats();
    spew_poll();
    tune_keys_poll();
    diag_keys_poll();
    drop_check_poll();
    note_frame_drawn_stats();
    trace_frame_boundary();


    // Pick the image the game drew most of its world into this frame.
    let (scene_fbo, scene_vp, had_world) = with_fs(|fs| {
        let had_world = !fs.counts.is_empty();
        if let Some((fbo, _)) = fs.counts.iter().max_by_key(|(_, n)| **n) {
            fs.scene_fbo = Some(*fbo);
        }
        if let Some(v) = fs.viewports.first() {
            fs.scene_vp = Some(*v);
        }
        fs.counts.clear();
        fs.viewports.clear();
        (fs.scene_fbo, fs.scene_vp, had_world)
    })
    .unwrap_or((None, None, true));

    // (blood is cleared by the game's own scene clear - see on_scene_cleared)
    let _ = had_world;

    let depth_works = DEPTH_COPY_STATE.load(Ordering::Relaxed) == 1 && DEPTH_ON.load(Ordering::Relaxed);
    let verts = {
        let mut sim = match SIM.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        let t_step = std::time::Instant::now();
        step_sim(&mut sim, depth_works);
        let ms = t_step.elapsed().as_secs_f64() * 1000.0;
        if let Ok(mut st) = SIM_TIME.lock() {
            st.0 += ms;
            st.1 = st.1.max(ms);
            st.2 += 1;
        }
        FLYING_NOW.store(sim.blobs.iter().filter(|b| !b.landed).count() as u32, Ordering::Relaxed);
        set_stage(0); // (whichever way step_sim returned)
        const CORNERS: [[f32; 2]; 6] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let t_verts = std::time::Instant::now();
        let mut v: Vec<f32> = Vec::with_capacity(sim.blobs.len() * 54);
        let stains_in_game = INJECTED_SHADERS.load(Ordering::Relaxed) > 0;

        for b in sim.blobs.iter() {
            if b.landed && stains_in_game {
                continue; // the game's own shaders draw landed blood now
            }
            let r = if b.landed { -b.radius } else { b.radius };
            // a rope drop is drawn round; the connector pieces run along the rope
            let dv = if !b.landed && b.group != u32::MAX && b.group & CAST_OFF_GROUP != 0 { [0.0; 3] } else { b.vel };
            for c in CORNERS.iter() {
                v.extend_from_slice(&[b.pos[0], b.pos[1], b.pos[2], r, c[0], c[1], dv[0], dv[1], dv[2]]);
            }
        }
        // Cast-off ropes: between linked drops, connector pieces drawn with the same drop shader -
        // each centred on its piece of the link and "moving" along it just fast enough that the
        // shader's stretch (half-length = radius x VISUAL_SCALE x (1 + min(speed x STRETCH, 2)))
        // spans it, so the drops join into one strand.
        {
            let mut r: Vec<(u32, u32, usize)> = sim
                .blobs
                .iter()
                .enumerate()
                .filter(|(_, b)| !b.landed && b.radius > 0.0 && b.group & CAST_OFF_GROUP != 0 && b.group != u32::MAX)
                .map(|(i, b)| (b.group, b.seq, i))
                .collect();
            r.sort_unstable();
            for w in r.windows(2) {
                if w[0].0 != w[1].0 || w[1].1 != w[0].1 + 1 {
                    continue;
                }
                let (a, b) = (&sim.blobs[w[0].2], &sim.blobs[w[1].2]);
                let d = sub3(b.pos, a.pos);
                let len = dot3(d, d).sqrt();
                if len < 0.01 || len > CAST_OFF_SPACING * 4.0 {
                    continue; // broken link: not drawn
                }
                let dir = [d[0] / len, d[1] / len, d[2] / len];
                let rc = a.radius.min(b.radius) * 0.85;
                let half = rc * VISUAL_SCALE;
                let pieces = ((len * 0.5) / (half * 3.0)).ceil().max(1.0);
                let seg = len / pieces;
                let s_need = (seg * 0.5) / half;
                let sp = if s_need > 1.0 { (s_need - 1.0) / STRETCH } else { 2.0 };
                for j in 0..pieces as usize {
                    let t = (j as f32 + 0.5) / pieces;
                    let c0 = [a.pos[0] + d[0] * t, a.pos[1] + d[1] * t, a.pos[2] + d[2] * t];
                    for c in CORNERS.iter() {
                        v.extend_from_slice(&[c0[0], c0[1], c0[2], rc, c[0], c[1], dir[0] * sp, dir[1] * sp, dir[2] * sp]);
                    }
                }
            }
        }
        let ms = t_verts.elapsed().as_secs_f64() * 1000.0;
        if let Ok(mut vt) = VERT_TIME.lock() {
            vt.0 += ms;
            vt.1 = vt.1.max(ms);
            vt.2 += 1;
        }
        v
    };

    let vp_now = current_view_proj();
    let vp_draw = match (vp_now, VP_HISTORY.lock()) {
        (Some(now), Ok(mut hist)) => {
            hist.push(now);
            while hist.len() > 2 {
                hist.remove(0);
            }
            if DELAY_ON.load(Ordering::Relaxed) { Some(hist[0]) } else { Some(now) }
        }
        (v, _) => v,
    };

    if frame % 600 == 0 {
        log::debug!(
            "Cruor: {} blobs, landed on surfaces {}, measured later {}, wall splats {}, on objects {}, ran off bodies {}, on fallback floor {}, floor correction {:.0} cm, depth copy {}",
            verts.len() / 54,
            DEPTH_LANDINGS.load(Ordering::Relaxed),
            RESOLVED_LANDINGS.load(Ordering::Relaxed),
            WALL_HITS.load(Ordering::Relaxed),
            OBJECT_HITS.load(Ordering::Relaxed),
            BODY_HITS.load(Ordering::Relaxed),
            FLOOR_LANDINGS.load(Ordering::Relaxed),
            floor_bias().0,
            match DEPTH_COPY_STATE.load(Ordering::Relaxed) {
                1 => "working",
                2 => "FAILED",
                _ => "not tried yet",
            }
        );
    }

    if let Ok(mut lv) = LAST_VERTS.lock() {
        lv.clear();
        lv.extend_from_slice(&verts);
    }
    // Drawn inside the game's scene lately? Then the overlay only updates the stain maps.
    let isf = IN_SCENE_FRAME.load(Ordering::Relaxed);
    let in_scene = isf != 0 && frame.wrapping_sub(isf) <= 2 && !IN_SCENE_FAILED.load(Ordering::Relaxed);
    let empty: Vec<f32> = Vec::new();
    let _ = in_scene;
    let overlay_verts = &empty; // TEST BUILD: no overlay fallback - flying blood is in-scene or nowhere
    // (screen_draw also keeps the in-game stain map up to date, so call it even with no drops.)
    if let (Some(draw), Some(now)) = (vp_draw, vp_now) {
        unsafe { screen_draw(overlay_verts, &draw, &now, scene_fbo, scene_vp) };
    }
}


// ============================================================
// Hooks into the game
// ============================================================
#[plugin(id = "com.penni.blood")]
mod plugin {
    // Damage function: 4 arguments, passed through untouched.
    #[hook_signature]
    extern "C" fn proc_dmg_and_stamina(
        motile_ptr: *mut std::ffi::c_void,
        damage: f32,
        injury: f32,
        can_kill: bool,
    ) -> std::ffi::c_char {
        register!("53 56 48 8D 64 24 D8 48 89 CB 40 30 F6 8B 05 ?? ?? ?? ?? 89");
        proc_dmg_and_stamina(motile_ptr, damage, injury, can_kill)
    }

    // The game's "spawn a blood spray" function: wound position, direction,
    // amount, and the character who is bleeding. With our blood on, we launch
    // our liquid instead of the game's spray; with it off (F5) the game's own
    // blood is used.
    #[hook_signature]
    extern "C" fn spawn_blood_spray(
        manager: *mut std::ffi::c_void,
        pos: *const f32,
        dir: *const f32,
        amount: f32,
        motile_ptr: *mut std::ffi::c_void,
    ) {
        register!("55 48 89 E5 48 8D A4 24 50 FF FF FF 48 89 9D 78 FF FF FF 48 89 7D 80 48 89 75 88 66 0F 7F 75 90 48 89 CB 0F 28 F3 48 8B");

        if !super::BLOOD_ON.load(std::sync::atomic::Ordering::Relaxed) || pos.is_null() {
            spawn_blood_spray(manager, pos, dir, amount, motile_ptr);
            return;
        }
        let p = unsafe { [pos.read_unaligned(), pos.add(1).read_unaligned(), pos.add(2).read_unaligned()] };
        let d = if dir.is_null() {
            [0.0, 1.0, 0.0]
        } else {
            unsafe { [dir.read_unaligned(), dir.add(1).read_unaligned(), dir.add(2).read_unaligned()] }
        };
        let floor_y = if motile_ptr.is_null() {
            p[1] - super::HIP_HEIGHT
        } else {
            let hip_y = unsafe { super::read_hip(motile_ptr as usize)[1] };
            hip_y - super::HIP_HEIGHT
        };
        let hard = super::hit_hardness(amount, (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt());
        super::note_spray(motile_ptr as usize, p);
        super::spawn_jet(p, d, amount, floor_y, hard);
        super::start_drip(motile_ptr as usize, p, hard);
    }


    // The game's OpenGL function loader. We swap in three small wrappers so we
    // can see the camera matrices as the game draws.
    #[hook_signature]
    extern "C" fn gl_get_proc(name: *const std::ffi::c_char) -> *mut std::ffi::c_void {
        register!("53 56 48 8D 64 24 D8 48 89 CB 48 89 DA 48 8B 0D ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 89 C6 48 85 C0 75");

        let ptr = gl_get_proc(name);
        if name.is_null() || ptr.is_null() {
            return ptr;
        }
        let n = unsafe { std::ffi::CStr::from_ptr(name) }.to_bytes();
        match n {
            b"glGetUniformLocation" => {
                super::REAL_GET_UNIFORM_LOCATION.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_get_uniform_location as *const () as usize as *mut std::ffi::c_void
            }
            b"glActiveTexture" => {
                super::REAL_ACTIVE_TEXTURE.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_active_texture as *const () as usize as *mut std::ffi::c_void
            }
            b"glFramebufferRenderbuffer" => {
                super::REAL_FB_RENDERBUFFER.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_fb_renderbuffer as *const () as usize as *mut std::ffi::c_void
            }
            b"glFramebufferTexture2D" => {
                super::REAL_FB_TEXTURE_2D.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_fb_texture_2d as *const () as usize as *mut std::ffi::c_void
            }
            b"glCopyTexSubImage2D" => {
                super::REAL_COPY_TEX_SUB.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_copy_tex_sub_image as *const () as usize as *mut std::ffi::c_void
            }
            b"glCopyTexImage2D" => {
                super::REAL_COPY_TEX.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_copy_tex_image as *const () as usize as *mut std::ffi::c_void
            }
            b"glGenerateMipmap" => {
                super::REAL_GEN_MIPMAP.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_generate_mipmap as *const () as usize as *mut std::ffi::c_void
            }
            b"glBindFramebufferEXT" => {
                super::REAL_BIND_FB_EXT.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_bind_framebuffer_ext as *const () as usize as *mut std::ffi::c_void
            }
            b"glBlitFramebufferEXT" => {
                super::REAL_BLIT_EXT.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_blit_ext as *const () as usize as *mut std::ffi::c_void
            }
            b"glDrawElementsInstanced" => {
                super::REAL_DRAW_ELEMENTS_INST.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_draw_elements_inst as *const () as usize as *mut std::ffi::c_void
            }
            b"glDrawArraysInstanced" => {
                super::REAL_DRAW_ARRAYS_INST.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_draw_arrays_inst as *const () as usize as *mut std::ffi::c_void
            }
            b"glClear" => {
                super::REAL_CLEAR.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_clear as *const () as usize as *mut std::ffi::c_void
            }
            b"glBlitFramebuffer" => {
                super::REAL_BLIT.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_blit as *const () as usize as *mut std::ffi::c_void
            }
            b"glBindTexture" => {
                super::REAL_BIND_TEXTURE.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_bind_texture as *const () as usize as *mut std::ffi::c_void
            }
            b"glDrawArrays" => {
                super::REAL_DRAW_ARRAYS.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_draw_arrays as *const () as usize as *mut std::ffi::c_void
            }
            b"glMultiDrawElements" => {
                super::REAL_MULTI_ELEMENTS.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_multi_draw_elements as *const () as usize as *mut std::ffi::c_void
            }
            b"glMultiDrawArrays" => {
                super::REAL_MULTI_ARRAYS.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_multi_draw_arrays as *const () as usize as *mut std::ffi::c_void
            }
            b"glDeleteProgram" => {
                super::REAL_DELETE_PROGRAM.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_delete_program as *const () as usize as *mut std::ffi::c_void
            }
            b"glLinkProgram" => {
                super::REAL_LINK_PROGRAM.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_link_program as *const () as usize as *mut std::ffi::c_void
            }
            b"glDeleteShader" => {
                super::REAL_DELETE_SHADER.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_delete_shader as *const () as usize as *mut std::ffi::c_void
            }
            b"glAttachShader" => {
                super::REAL_ATTACH_SHADER.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_attach_shader as *const () as usize as *mut std::ffi::c_void
            }
            b"glUniform1i" => {
                super::REAL_UNIFORM1I.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_uniform1i as *const () as usize as *mut std::ffi::c_void
            }
            b"glBindVertexArray" => {
                super::REAL_BIND_VAO.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_bind_vertex_array as *const () as usize as *mut std::ffi::c_void
            }
            b"glDrawElements" => {
                super::REAL_DRAW_ELEMENTS.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_draw_elements as *const () as usize as *mut std::ffi::c_void
            }
            b"glDrawRangeElements" => {
                super::REAL_DRAW_RANGE.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_draw_range_elements as *const () as usize as *mut std::ffi::c_void
            }
            b"glShaderSource" => {
                super::REAL_SHADER_SOURCE.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_shader_source as *const () as usize as *mut std::ffi::c_void
            }
            b"glCompileShader" => {
                super::REAL_COMPILE_SHADER.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_compile_shader as *const () as usize as *mut std::ffi::c_void
            }
            b"glUseProgram" => {
                super::REAL_USE_PROGRAM.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_use_program as *const () as usize as *mut std::ffi::c_void
            }
            b"glBindFramebuffer" => {
                super::REAL_BIND_FRAMEBUFFER.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                info_f!("Cruor: in-scene drawing installed");
                super::my_bind_framebuffer as *const () as usize as *mut std::ffi::c_void
            }
            b"glViewport" => {
                super::REAL_VIEWPORT.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                super::my_viewport as *const () as usize as *mut std::ffi::c_void
            }
            b"glUniformMatrix4fv" => {
                super::REAL_UNIFORM_MATRIX4FV.store(ptr as usize, std::sync::atomic::Ordering::Relaxed);
                info_f!("Cruor: camera capture installed");
                super::my_uniform_matrix4fv as *const () as usize as *mut std::ffi::c_void
            }
            _ => ptr,
        }
    }

    // The game's "add a blood entry to a character" (0x100157D90 in 0.9.5.2d), called by
    // its damage handler (0x10015B5C8) right after the blood spray: (character, &world
    // position, hit bone, amount). Up to 7 entries per character (+0x89C count, 0x30-byte
    // entries from +0x8A0); each is applied 7 frames later to the body and worn items.
    // TEST: watched only.
    #[hook_signature]
    extern "C" fn add_blood_entry(ch: *mut std::ffi::c_void, pos: *const f32, bone: *mut std::ffi::c_void, amount: f32) {
        register!("53 48 8D 64 24 C0 66 0F 7F 74 24 30 48 89 CB 0F 28 F3 48 8B 02 48 89 44 24 20");
        super::on_add_blood_entry(ch as usize, pos, bone as usize, amount);
        add_blood_entry(ch, pos, bone, amount)
    }

    // The game's per-character hit processor (0x100157B50 in 0.9.5.2d): character in rcx;
    // reads the hit record the game stored on the character (+0x8A0) and bloodies the
    // body (0x1000D2470) and attached items (0x100062DC0). Called once per character per
    // frame from 0x100168195; returns nothing. TEST: watched only.
    #[hook_signature]
    extern "C" fn hit_processor(ch: *mut std::ffi::c_void) {
        register!("53 57 56 41 54 41 55 48 8D 64 24 C0 48 89 CB 8B 83 ?? ?? 00 00");
        super::on_hit_processor(ch as usize);
        hit_processor(ch)
    }

    // The game's per-character update (TMotileDynamics method, every character, every update,
    // on the game's thread; character in rcx only). It calls the hit processor only when the
    // character has pending blood entries - so queued drop hits are handed over here instead.
    #[hook_signature]
    extern "C" fn char_update(ch: *mut std::ffi::c_void) {
        register!("55 48 89 E5 48 8D A4 24 F0 FE FF FF 48 89 9D 20 FF FF FF 48 89 BD 28 FF FF FF");
        super::on_char_update(ch as usize);
        char_update(ch)
    }

    // The game's "put blood on this mesh around a point" (0x100062DC0 in 0.9.5.2d).
    // Called (via a queued record, handler 0x100157AB0) for every mesh part of a
    // character's model when the character is hit: (part, &position, &direction, amount).
    // TEST: watched only - the first calls are logged, nothing changed.
    #[hook_signature]
    extern "C" fn blood_on_mesh(part: *mut std::ffi::c_void, pos: *const f32, dir: *const f32, amount: f32) {
        register!("53 57 56 41 54 41 55 41 56 41 57 48 8D A4 24 60 FF FF FF 66 0F 7F 74 24 50");
        super::note_blood_on_mesh(part as usize, pos, dir, amount);
        blood_on_mesh(part, pos, dir, amount)
    }

    // The game's scene clear (0x10007D4D0 in 0.9.5.2d): empties the scene's object list
    // and its other lists. Called by every level load / reset path (the .rfc level
    // loader, the arena's next fight, ...), never during normal play. This is the
    // moment the level is unloaded.
    #[hook_signature]
    extern "C" fn scene_clear(scene: *mut std::ffi::c_void) {
        register!("53 57 56 48 8D 64 24 D0 48 89 CB 48 8D 8B 70 1C 00 00 48 8B 15 ?? ?? ?? ?? E8");
        super::on_scene_cleared(scene as usize);
        scene_clear(scene)
    }

    // The game's sphere-cast-through-the-world function (used by its camera).
    // We watch it to learn where the game keeps its world ("scene"), and as the
    // sign that the game is using the level (it only runs while the level is live).
    #[hook_signature]
    extern "C" fn world_intersect(
        origin: *const f32,
        dir: *const f32,
        max_dist: f32,
        radius: f32,
        scene: *mut std::ffi::c_void,
        out: *mut f32,
    ) -> u8 {
        register!("55 48 89 E5 48 8D A4 24 A0 FE FF FF 48 89 9D C0 FE FF FF 48 89 BD C8 FE FF FF 48 89 B5 D0 FE FF FF 4C 89 A5 D8 FE FF FF 48 89 4D F8");
        if !scene.is_null() {
            let old = super::SCENE.swap(scene as usize, std::sync::atomic::Ordering::Relaxed);
            if old == 0 {
                info_f!("Cruor: found the game's collision world");
            }
            super::WORLD_LIVE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        world_intersect(origin, dir, max_dist, radius, scene, out)
    }


    // Lower the "is this hit bad enough to bleed?" threshold from 0.05 to 0.01.
    #[patch_signature(offset = 12)]
    fn lower_blood_threshold(address: *mut u8) -> Vec<u8> {
        register!("F3 0F 5A 85 68 FF FF FF 66 0F 2F 05 ?? ?? ?? ?? 0F 8A ?? ?? ?? ?? 0F 86 ?? ?? ?? ?? F3 0F 5A 85 68 FF FF FF 66 0F 2F 05");

        unsafe {
            let old_disp = (address as *const i32).read_unaligned();
            let next_ip = address.add(4);
            let old_target = next_ip.offset(old_disp as isize) as *const f64;
            let new_disp = old_disp - 0xD0;
            let new_target = next_ip.offset(new_disp as isize) as *const f64;
            if (old_target.read_unaligned() - 0.05).abs() < 1e-9 && (new_target.read_unaligned() - 0.01).abs() < 1e-9 {
                new_disp.to_le_bytes().to_vec()
            } else {
                warn_f!("Cruor threshold NOT changed (game version differs?)");
                old_disp.to_le_bytes().to_vec()
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn enable() {
    FIRST_RUN.get_or_init(|| {
        pretty_env_logger::formatted_builder()
            .filter_level(LevelFilter::Info)
            .init();
        true
    });
    load_settings();

    // Find where the game keeps the player and its frame function, by reading them
    // out of the game's own code (so the mod keeps working across small updates,
    // and switches itself off cleanly if an update changes things).
    unsafe {
        match find_pattern(PLAYER_SLOT_SIG) {
            Some(a) => {
                PLAYER_SLOT.store(rip_target(a + 10, a + 14), Ordering::Relaxed);
                let field = ((a + 2) as *const u32).read_unaligned();
                let layout = match field {
                    0xE68 => Some(("0.9.5.2 public", 0xB0, 0x89C, 0x8A0, 0x12F0, 0x5F0, 0x360)),
                    0xD58 => Some(("new beta", 0xF0, 0x78C, 0x790, 0x11E0, 0x4D0, 0x240)),
                    _ => None,
                };
                match layout {
                    Some((name, c, n, arr, it, sec, pos)) => {
                        OFF_POS.store(pos, Ordering::Relaxed);
                        OFF_CORNERS.store(c, Ordering::Relaxed);
                        OFF_BLOOD_N.store(n, Ordering::Relaxed);
                        OFF_BLOOD_ARR.store(arr, Ordering::Relaxed);
                        OFF_ITEMS.store(it, Ordering::Relaxed);
                        OFF_SECTOR.store(sec, Ordering::Relaxed);
                        LAYOUT_OK.store(true, Ordering::Relaxed);
                        info_f!("Cruor: character layout of the {} version", name);
                    }
                    None => warn_f!("Cruor: unknown character layout (0x{:X}) - blood on characters is switched off", field),
                }
            }
            None => warn_f!("Cruor: couldn't find the player in this Exanima version - the F10 stream and blood on characters won't work"),
        }
        match find_pattern(SWAP_SLOT_SIG) {
            Some(a) => SWAP_SLOT.store(rip_target(a + 8, a + 13), Ordering::Relaxed),
            None => warn_f!("Cruor: this Exanima version isn't supported (frame function not found) - blood is switched off, the game is unaffected"),
        }
        // Both must lie inside the game's own exe, or we don't use them.
        let base = GetModuleHandleA(std::ptr::null()) as usize;
        let e_lfanew = ((base + 0x3C) as *const u32).read_unaligned() as usize;
        let size = ((base + e_lfanew + 0x18 + 0x38) as *const u32).read_unaligned() as usize;
        for slot in [&PLAYER_SLOT, &SWAP_SLOT] {
            let v = slot.load(Ordering::Relaxed);
            if v != 0 && (v < base || v >= base + size) {
                slot.store(0, Ordering::Relaxed);
                warn_f!("Cruor: a game address looked wrong for this version; that feature is switched off");
            }
        }
        if PLAYER_SLOT.load(Ordering::Relaxed) != 0 && SWAP_SLOT.load(Ordering::Relaxed) != 0 {
            info_f!("Cruor: this Exanima version is supported");
        }
    }

    // Find the collision function before our hooks change its first bytes.
    if WORLD_INTERSECT.load(Ordering::Relaxed) == 0 {
        match unsafe { find_pattern(INTERSECT_SIG) } {
            Some(addr) => {
                WORLD_INTERSECT.store(addr, Ordering::Relaxed);
                info_f!("Cruor: collision function at 0x{:X}", addr);
            }
            None => warn_f!("Cruor: collision function NOT found"),
        }
    }

    let mut plugin = unsafe { plugin::get() };
    if let Err(error) = unsafe { plugin.on_enable() } {
        error_f!("Error while enabling plugin: {:?}", error);
    }
    file_log("Cruor 1.0.0 log started");
    info_f!("Cruor 1.0.0 enabled. F5 blood on/off, F8 clear, hold F10 for a blood stream, Numpad 8/2 pick a setting, 4/6 change, 5 reset");
}

#[no_mangle]
pub extern "C" fn disable() {
    let mut plugin = unsafe { plugin::get() };
    if let Err(error) = unsafe { plugin.on_disable() } {
        error_f!("Error while disabling plugin: {:?}", error);
    }
    info_f!("Cruor plugin disabled");
}

#[no_mangle]
pub unsafe extern "C" fn setting_changed_bool(name: char_p::Box, value: bool) {
    let mut plugin = unsafe { plugin::get() };
    unsafe {
        plugin.on_setting_changed_bool(name, value, |key, value| {
            debug!("Setting changed: {} = {}", key, value);
        })
    };
}

#[no_mangle]
pub extern "C" fn setting_changed_int(name: char_p::Box, value: i32) {
    debug!("Setting changed: {} = {}", name.to_string(), value);
}

#[no_mangle]
pub extern "C" fn setting_changed_float(name: char_p::Box, value: f32) {
    debug!("Setting changed: {} = {}", name.to_string(), value);
}

#[no_mangle]
pub extern "C" fn setting_changed_string(name: char_p::Box, value: char_p::Box) {
    debug!("Setting changed: {} = {}", name.to_string(), value.to_string());
}
