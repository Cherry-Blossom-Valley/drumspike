//! P3-15c: an open, `project_get` and `tracks_get` read each unchanged source head once (the
//! head-hash cache of the core, keyed by device, inode, size, mtime and ctime). One test in its own
//! binary: the cache counters are global to the process.

mod common;

use std::time::Duration;

use common::*;
use drumspike_core::project::head_cache_counts;

#[test]
fn project_open_get_and_tracks_get_hash_each_unchanged_source_once() {
    let root = tmp("headcache");
    let dir = project(
        &root,
        "Song",
        &[
            kick_spec("Kick_In", "kick", 36),
            kick_spec("Snare_Top", "snare", 38),
            kick_spec("Tom_1", "tom", 45),
        ],
    );
    // a head is kept only when the file did not change for HEAD_CACHE_SETTLE (3 s)
    std::thread::sleep(drumspike_core::project::HEAD_CACHE_SETTLE + Duration::from_millis(200));
    let svc = service_dsp();
    let c0 = head_cache_counts();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let c1 = head_cache_counts();
    assert_eq!(c1.hashed - c0.hashed, 3, "the open reads each head once: {c0:?} {c1:?}");
    for _ in 0..3 {
        let g = svc.project_get(&sid).unwrap();
        assert!(
            g["tracks"].as_object().unwrap().values().all(|t| t["status"] == "ok"),
            "{g}"
        );
        let t = svc.tracks_get(&sid).unwrap();
        assert!(t["tracks"].as_object().unwrap().values().all(|t| t["offline"] == false));
    }
    let c2 = head_cache_counts();
    assert_eq!(c2.hashed, c1.hashed, "project_get and tracks_get read no head again");
    assert!(c2.hits - c1.hits >= 18, "{c1:?} {c2:?}");
    // a source that changes (same size, other bytes) is read again and goes offline
    let wav = root.join("daw").join("Tom_1.wav");
    let mut bytes = std::fs::read(&wav).unwrap();
    let n = bytes.len();
    bytes[n / 2] ^= 0xff;
    bytes[100] ^= 0xff; // inside the first MiB: the head hash differs
    std::fs::write(&wav, &bytes).unwrap();
    let t = svc.tracks_get(&sid).unwrap();
    assert_eq!(t["tracks"]["Tom_1"]["status"], "changed");
    assert_eq!(t["tracks"]["Kick_In"]["status"], "ok");
    let c3 = head_cache_counts();
    assert!(c3.hashed > c2.hashed, "the changed file is read again");
    std::fs::remove_dir_all(&root).unwrap();
}
