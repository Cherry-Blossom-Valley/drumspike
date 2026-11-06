//! The refine golden (refactor step 2.4, R5): the exact bits of every refined hit on a fixed synthetic corpus, for the
//! 7 rules, two back-track peak margins and the lead-in switch off and on, plus the sample-bound walk
//! (`first_motion_mid`) and its trace (`first_motion_trace`). A move or a parameter object in `refine` must not change
//! one bit. The values are taken on an Apple-silicon Mac: the FFT of the reference rule may round differently with
//! another SIMD path, and the corpus uses the platform's `sin` and `exp`, so other targets skip the comparison.

use std::process::Command;
use std::sync::atomic::Ordering;

use super::walk::LEAD_IN;
use super::*;

/// The lower peak margin of the golden (the margin the app's edit tests use beside [`FM_PEAK_DB`]).
const LOW_MARGIN_DB: f64 = 29.0;
/// Set in the child process that runs the lead-in leg alone (the switch is process-wide).
const CHILD_ENV: &str = "REFINE_GOLDEN_LEAD_IN_CHILD";

/// FNV-1a over 64-bit words.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn word(&mut self, w: u64) {
        for b in w.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn reach_code(s: ReachStep) -> u64 {
    match s {
        ReachStep::Kept => 0,
        ReachStep::Reached => 1,
        ReachStep::Tail => 2,
        ReachStep::Unresolved => 3,
    }
}

fn back_code(s: BackStep) -> u64 {
    match s {
        BackStep::Off => 0,
        BackStep::Moved => 1,
        BackStep::Kept => 2,
        BackStep::Busy => 3,
        BackStep::Separate => 4,
    }
}

fn hash_refined(h: &mut Fnv, r: &Refined) {
    h.word(r.t.to_bits());
    match r.rise {
        Some(v) => {
            h.word(1);
            h.word(v.to_bits());
        }
        None => h.word(0),
    }
    h.word(reach_code(r.step));
    h.word(back_code(r.back));
}

/// The kick of `tests/helpers.kick_hit` at rate `sr`.
fn kick_hit(sr: u32, n: usize, attack_ms: f64) -> Vec<f64> {
    let srf = f64::from(sr);
    let mut cum = 0.0;
    (0..n)
        .map(|i| {
            let t = i as f64 / srf;
            cum += 55.0 + 70.0 * (-t / 0.03).exp();
            let env = (t / (attack_ms / 1000.0)).min(1.0) * (-t / 0.12).exp();
            (2.0 * std::f64::consts::PI * cum / srf).sin() * env
        })
        .collect()
}

/// Kicks at `(time s, level dB, attack ms)` over `dur` seconds.
fn kicks(sr: u32, hits: &[(f64, f64, f64)], dur: f64) -> Vec<f32> {
    let srf = f64::from(sr);
    let mut x = vec![0.0f64; (dur * srf) as usize];
    for &(t, db, att) in hits {
        let i = (t * srf).round() as usize;
        let g = 10f64.powf(db / 20.0);
        for (k, v) in kick_hit(sr, (0.4 * srf) as usize, att).iter().enumerate() {
            if i + k < x.len() {
                x[i + k] += v * g;
            }
        }
    }
    x.iter().map(|&v| v as f32).collect()
}

/// A uniform noise floor of amplitude `a` (a fixed LCG seed).
fn floor_of(n: usize, a: f64, seed: u64) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (a * ((s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0)) as f32
        })
        .collect()
}

/// Adds a decaying sine of `hz` and peak `amp` from sample `from`, with a raised-cosine start of `ramp` samples and a
/// start phase `ph`.
fn swing(x: &mut [f32], sr: u32, from: usize, ramp: usize, amp: f64, hz: f64, ph: f64) {
    let srf = f64::from(sr);
    for k in 0..(0.1 * srf) as usize {
        if from + k >= x.len() {
            break;
        }
        let t = k as f64 / srf;
        let env = if k >= ramp {
            1.0
        } else {
            0.5 - 0.5 * (std::f64::consts::PI * k as f64 / ramp as f64).cos()
        };
        x[from + k] += (amp * env * (-t / 0.06).exp() * (2.0 * std::f64::consts::PI * hz * t + ph).sin()) as f32;
    }
}

/// A sine of `amp` and `f` Hz in `[from, to)`.
fn tone(x: &mut [f32], sr: u32, from: usize, to: usize, f: f64, amp: f64) {
    for (k, v) in x[from..to].iter_mut().enumerate() {
        *v += (amp * (2.0 * std::f64::consts::PI * f * k as f64 / f64::from(sr)).sin()) as f32;
    }
}

/// A straight ramp of `amp` over `len` samples from `from`.
fn drift(x: &mut [f32], from: usize, len: usize, amp: f64) {
    for k in 0..len {
        x[from + k] += (amp * k as f64 / len as f64) as f32;
    }
}

/// One signal of the corpus: the rate, the samples, the attack onsets (seconds) and the coarse times (seconds,
/// ascending) the track entry points refine.
struct Case {
    sr: u32,
    x: Vec<f32>,
    onsets: Vec<f64>,
    coarse: Vec<f64>,
}

fn case(sr: u32, x: Vec<f32>, onsets: Vec<f64>) -> Case {
    // the detector reports about 6 ms early; one late time and one time in a gap test the other branches
    let mut coarse: Vec<f64> = onsets.iter().map(|&t| t - 0.006).collect();
    let dur = x.len() as f64 / f64::from(sr);
    coarse.push(dur - 0.05);
    coarse.sort_by(f64::total_cmp);
    Case { sr, x, onsets, coarse }
}

fn corpus() -> Vec<Case> {
    let mut out = Vec::new();
    // kicks with close pairs, a quiet hit and a slow attack, at three rates; one copy over a noise floor
    let hits = [
        (0.10, -6.0, 0.5),
        (0.35, -20.0, 0.5),
        (0.362, -3.0, 0.5),
        (0.70, -40.0, 4.0),
        (1.00, -1.0, 0.2),
        (1.12, -9.0, 8.0),
    ];
    for sr in [44_100u32, 48_000, 96_000] {
        let x = kicks(sr, &hits, 1.5);
        out.push(case(sr, x, hits.iter().map(|h| h.0).collect()));
    }
    let mut x = kicks(48_000, &hits, 1.5);
    for (v, n) in x.iter_mut().zip(floor_of(72_000, 1e-3, 0x9E37_79B9_7F4A_7C15)) {
        *v += n;
    }
    out.push(case(48_000, x, hits.iter().map(|h| h.0).collect()));

    // hits with a lead-in, a first bump, a long rise, a drift, bleed and a hard half-sample start over a floor
    let sr = 48_000u32;
    let mut x = floor_of(96_000, 0.0017, 0x2545_F491_4F6C_DD1D);
    let mut on = Vec::new();
    // a lead-in 31 dB under the hit for the 3 ms before it
    tone(&mut x, sr, 9_856, 10_000, 900.0, 0.028);
    swing(&mut x, sr, 10_000, 0, 1.0, 180.0, 0.7);
    on.push(10_000);
    // a small first bump, the main swing 1 ms later
    swing(&mut x, sr, 20_000, 24, 0.01, 1000.0, 0.7);
    swing(&mut x, sr, 20_048, 0, 0.8, 180.0, 0.7);
    on.push(20_000);
    // the same bump 3 ms before the swing
    swing(&mut x, sr, 30_000, 24, 0.01, 1000.0, 0.7);
    swing(&mut x, sr, 30_144, 0, 0.8, 180.0, 0.7);
    on.push(30_000);
    // a slow drift before a snare stroke
    drift(&mut x, 39_900, 100, 0.02);
    swing(&mut x, sr, 40_000, 6, 0.9, 200.0, 0.0);
    on.push(40_000);
    // bleed of another drum 40 dB under the hit for 8 ms before it
    tone(&mut x, sr, 49_616, 50_000, 60.0, 0.009);
    swing(&mut x, sr, 50_000, 12, 0.9, 220.0, 0.3);
    on.push(50_000);
    // a slow attack without a steep edge
    swing(&mut x, sr, 60_000, 480, 0.7, 150.0, 0.0);
    on.push(60_000);
    // a hard start at full level, half a sample late (sinc pre-ringing)
    let mut hit = vec![0.0f32; 96_000];
    swing(&mut hit, sr, 70_000, 0, 0.9, 180.0, 1.2);
    for i in 1..hit.len() {
        x[i] += 0.5 * (hit[i] + hit[i - 1]);
    }
    on.push(70_000);
    // a quiet event 15 dB under a hit 4 ms after it, and a decaying tail 60 ms before a hit
    swing(&mut x, sr, 79_808, 4, 0.16, 300.0, 0.0);
    swing(&mut x, sr, 80_000, 4, 0.9, 180.0, 0.0);
    on.push(80_000);
    swing(&mut x, sr, 87_120, 2, 0.5, 120.0, 0.0);
    swing(&mut x, sr, 90_000, 2, 0.6, 180.0, 0.0);
    on.push(87_120);
    on.push(90_000);
    out.push(case(sr, x, on.iter().map(|&s| s as f64 / f64::from(sr)).collect()));

    // a snare roll at 75 ms: the tail of the first hit is above 25 % of the second (the tail fix)
    let mut buf = vec![0.0f64; 48_000];
    let mut s: u64 = 12345;
    for onset in [0.300, 0.375] {
        let i0 = (onset * 48_000.0) as usize;
        for k in 0..19_200 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let u = (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            let t = k as f64 / 48_000.0;
            let tone = (0.7 + 2.0 * std::f64::consts::PI * 190.0 * t).sin() * (-t / 0.07).exp();
            buf[i0 + k] += 0.4 * tone + 0.5 * u * (-t / 0.11).exp();
        }
    }
    out.push(case(
        48_000,
        buf.iter().map(|&v| v as f32).collect(),
        vec![0.300, 0.375],
    ));

    // 7 ms of a tone joined to a hit: 20 dB under it a separate earlier event, 3 dB under it the hit's own body
    let mut x = vec![0.0f32; 48_000];
    tone(&mut x, 48_000, 11_664, 12_000, 900.0, 0.1);
    swing(&mut x, 48_000, 12_000, 0, 1.0, 180.0, 0.7);
    tone(&mut x, 48_000, 35_664, 36_000, 900.0, 0.7);
    swing(&mut x, 48_000, 36_000, 0, 1.0, 180.0, 0.7);
    out.push(case(48_000, x, vec![0.25, 0.75]));

    // a silent signal and a very short one (the early returns)
    out.push(case(48_000, vec![0.0; 24_000], vec![0.2, 0.3]));
    out.push(Case {
        sr: 48_000,
        x: vec![0.1; 6],
        onsets: vec![0.0],
        coarse: vec![0.0, 0.0001],
    });
    out
}

/// The golden hashes of one lead-in state: one per (rule, margin), then the sample-bound walk and its trace.
#[derive(Debug, PartialEq, Eq)]
struct Golden {
    rules: Vec<(u32, u64, u64)>,
    mid: u64,
    trace: u64,
    /// Reach and back steps seen, by code (the corpus must reach every branch).
    reach_seen: [bool; 4],
    back_seen: [bool; 5],
    /// Traces whose lead-in moved the foot.
    lead_moved: usize,
}

fn compute() -> Golden {
    let corpus = corpus();
    let rules = [
        RefineRule::Reference,
        RefineRule::Reach,
        RefineRule::FirstMotion,
        RefineRule::Foot,
        RefineRule::FootCapped,
        RefineRule::Edge,
        RefineRule::Visible,
        RefineRule::Line,
        RefineRule::Departure,
    ];
    let mut reach_seen = [false; 4];
    let mut back_seen = [false; 5];
    let mut out = Vec::new();
    for rule in rules {
        let mut per_margin = [0u64; 2];
        for (mi, margin) in [FM_PEAK_DB, LOW_MARGIN_DB].into_iter().enumerate() {
            let mut h = Fnv::new();
            for c in &corpus {
                for win in [RefineWindow::DEFAULT, RefineWindow::SYMMETRIC_7] {
                    let track = refine_track_rule_peak(&c.x, c.sr, &c.coarse, win, rule, margin);
                    h.word(track.len() as u64);
                    for r in &track {
                        hash_refined(&mut h, r);
                        reach_seen[reach_code(r.step) as usize] = true;
                        back_seen[back_code(r.back) as usize] = true;
                    }
                    // one hit at a time at several offsets, without neighbours and with a close one after it
                    for &on in &c.onsets {
                        for off_ms in [-9.0, -6.0, -2.0, 0.0, 3.0, 11.0] {
                            let t0 = on + off_ms / 1000.0;
                            for nb in [&[][..], &[t0 + 0.004][..], &[t0 - 0.03, t0 + 0.05][..]] {
                                let r = refine_rule_peak(&c.x, t0, c.sr, nb, win, rule, margin);
                                hash_refined(&mut h, &r);
                                reach_seen[reach_code(r.step) as usize] = true;
                                back_seen[back_code(r.back) as usize] = true;
                            }
                        }
                    }
                }
            }
            per_margin[mi] = h.0;
        }
        out.push((rule.version(), per_margin[0], per_margin[1]));
    }
    let mut mid = Fnv::new();
    let mut trace = Fnv::new();
    let mut lead_moved = 0;
    for c in &corpus {
        let srf = f64::from(c.sr);
        for &on in &c.onsets {
            let s0 = (on * srf).round() as i64;
            for ds in [-40i64, -3, 0, 2, 12, 60] {
                let s = s0 + ds;
                for (pm, nm) in [(None, None), (Some(s - 200), Some(s + 300)), (Some(s - 40), None)] {
                    let (foot, step) = first_motion_mid(&c.x, s, c.sr, pm, nm);
                    mid.word(foot as u64);
                    mid.word(back_code(step));
                    match first_motion_trace(&c.x, s, c.sr, pm, nm) {
                        Some(tr) => {
                            trace.word(1);
                            trace.word(tr.floor.to_bits());
                            trace.word(tr.thr.to_bits());
                            trace.word(tr.foot as u64);
                            trace.word(tr.lead as u64);
                            lead_moved += usize::from(tr.lead != tr.foot);
                        }
                        None => trace.word(0),
                    }
                }
            }
        }
    }
    Golden {
        rules: out,
        mid: mid.0,
        trace: trace.0,
        reach_seen,
        back_seen,
        lead_moved,
    }
}

/// The golden with the lead-in off (the product): (version, hash at [`FM_PEAK_DB`], hash at [`LOW_MARGIN_DB`]).
/// Version 9 (#351) is unrecorded: the values are taken on Apple silicon and #351 was built on another target;
/// the coordinator records them on the branch after `done` (the check prints them).
const UNRECORDED: u64 = 0;
const OFF: [(u32, u64, u64); 9] = [
    (1, 0xce84_2a40_9231_a3ef, 0xce84_2a40_9231_a3ef),
    (2, 0x0221_4015_c435_0d16, 0x0221_4015_c435_0d16),
    (3, 0x6d67_d448_07a8_c595, 0x7bd1_e669_8800_3c91),
    (4, 0x7d0c_8795_ab33_d108, 0x17d8_caa2_fee0_d5d9),
    (5, 0x1664_0503_cbdb_0e28, 0x5600_43ff_7834_30e9),
    (6, 0x841b_06b6_f77a_a3b8, 0x87d9_8aa9_1d81_ac23),
    (7, 0x1d41_975c_bdd7_495b, 0xbb3f_b37a_bb64_67f8),
    (8, UNRECORDED, UNRECORDED),
    (9, UNRECORDED, UNRECORDED),
];
const OFF_MID: u64 = 0x8555_f834_1234_4faf;
const OFF_TRACE: u64 = 0x4a00_4b51_e34f_2c4f;

/// The golden with the lead-in on (#239, no product path turns it on). Versions 8 and 9 unrecorded, as `OFF`.
const ON: [(u32, u64, u64); 9] = [
    (1, 0xce84_2a40_9231_a3ef, 0xce84_2a40_9231_a3ef),
    (2, 0x0221_4015_c435_0d16, 0x0221_4015_c435_0d16),
    (3, 0xb223_7b9c_5549_e49a, 0x3af3_178b_c6b6_e1a0),
    (4, 0x37bd_f1a6_9d96_26d1, 0xbd8c_8cff_36bf_02f1),
    (5, 0x4078_33ce_e8ee_3b12, 0x6bff_11ce_7cad_78f2),
    (6, 0x4ff8_a688_ac7b_8b33, 0x4621_c0d0_3737_3b5a),
    (7, 0x1301_7400_f0c2_8044, 0x9fd4_68ab_21e1_86e9),
    (8, UNRECORDED, UNRECORDED),
    (9, UNRECORDED, UNRECORDED),
];
const ON_MID: u64 = 0xd010_6fcc_4caa_fa2a;
const ON_TRACE: u64 = 0x63e1_48f2_a4d8_d425;

fn check(g: &Golden, rules: &[(u32, u64, u64); 9], mid: u64, trace: u64, label: &str) {
    let table: String = g
        .rules
        .iter()
        .map(|(v, a, b)| format!("    ({v}, 0x{a:016x}, 0x{b:016x}),\n"))
        .collect();
    let got = format!(
        "{label}:\n{table}MID: 0x{:016x}\nTRACE: 0x{:016x}\nlead_moved {}",
        g.mid, g.trace, g.lead_moved
    );
    assert_eq!(g.rules.len(), rules.len(), "{got}");
    for ((gv, ga, gb), (v, a, b)) in g.rules.iter().zip(rules.iter()) {
        assert_eq!(gv, v, "{got}");
        if *a == UNRECORDED && *b == UNRECORDED {
            // the entry was built on another target: print the values for the coordinator to record
            println!("version {gv} golden unrecorded, record on Apple silicon: 0x{ga:016x}, 0x{gb:016x}");
        } else {
            assert_eq!((ga, gb), (a, b), "{got}");
        }
    }
    assert_eq!((g.mid, g.trace), (mid, trace), "{got}");
}

#[test]
#[cfg_attr(
    not(all(target_arch = "aarch64", target_os = "macos")),
    ignore = "the golden bits are taken on Apple silicon"
)]
fn refine_golden_lead_in_off() {
    assert!(
        !LEAD_IN.load(Ordering::Relaxed),
        "the lead-in is off in the test process"
    );
    let g = compute();
    // the corpus reaches every step of both stages, so a change in any branch shows
    assert_eq!(g.reach_seen, [true; 4], "reach steps seen: {:?}", g.reach_seen);
    assert_eq!(g.back_seen, [true; 5], "back steps seen: {:?}", g.back_seen);
    assert_eq!(g.lead_moved, 0, "the lead-in is off");
    check(&g, &OFF, OFF_MID, OFF_TRACE, "lead-in off");
}

#[test]
#[cfg_attr(
    not(all(target_arch = "aarch64", target_os = "macos")),
    ignore = "the golden bits are taken on Apple silicon"
)]
fn refine_golden_lead_in_on() {
    if std::env::var_os(CHILD_ENV).is_some() {
        // the child process runs this test alone: the switch cannot reach another test
        LEAD_IN.store(true, Ordering::Relaxed);
        let g = compute();
        assert!(g.lead_moved > 0, "the lead-in moved no foot: the switch did not fire");
        check(&g, &ON, ON_MID, ON_TRACE, "lead-in on");
        // the switch changes the result of some rule (versions 3 to 5 walk with the lead-in); the recorded
        // entries only (8 and 9 are unrecorded and always differ from their placeholders)
        assert_ne!(&g.rules[..7], &OFF[..7], "the lead-in changed no rule");
        println!("refine golden lead-in child: checked");
        return;
    }
    let exe = std::env::current_exe().expect("the test binary");
    let out = Command::new(exe)
        .args([
            "--exact",
            "refine::golden_tests::refine_golden_lead_in_on",
            "--include-ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .expect("run the child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "child failed:\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("refine golden lead-in child: checked"),
        "the child did not run the check:\n{stdout}\n{stderr}"
    );
}
