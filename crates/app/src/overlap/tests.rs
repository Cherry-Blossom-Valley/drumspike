use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A prepared item that counts how many of its kind are alive.
struct Live<'a> {
    name: &'static str,
    counter: &'a Counter,
}

#[derive(Default)]
struct Counter {
    live: AtomicUsize,
    max: AtomicUsize,
}

impl Counter {
    fn make<'a>(&'a self, name: &'static str) -> Live<'a> {
        let n = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(n, Ordering::SeqCst);
        Live { name, counter: self }
    }
}

impl Drop for Live<'_> {
    fn drop(&mut self) {
        self.counter.live.fetch_sub(1, Ordering::SeqCst);
    }
}

fn wait_until(deadline_ms: u64, f: impl Fn() -> bool) -> bool {
    let t0 = Instant::now();
    while !f() {
        if t0.elapsed() > Duration::from_millis(deadline_ms) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    true
}

#[test]
fn the_pool_has_the_named_threads_and_runs_rayon_work() {
    let p = pool();
    assert_eq!(p.current_num_threads(), pool_threads());
    let d = default_threads();
    assert!((1..=8).contains(&d), "{d}");
    assert!(d <= crate::overlap::physical_cores());
    // the override is read when the pool is built; here only the parse is checked
    match std::env::var("DRUMSPIKE_PREPARE_THREADS") {
        Ok(v) => assert_eq!(pool_threads(), v.trim().parse::<usize>().unwrap_or(d).max(1)),
        Err(_) => assert_eq!(pool_threads(), d),
    }
    let names: Vec<String> = p.install(|| {
        use rayon::prelude::*;
        (0..64usize)
            .into_par_iter()
            .map(|_| std::thread::current().name().unwrap_or("?").to_string())
            .collect()
    });
    assert!(names.iter().all(|n| n.starts_with("ds-prepare-")), "{names:?}");
    assert!(std::ptr::eq(p, pool()), "one pool per process");
}

#[test]
fn items_are_prepared_one_ahead_and_consumed_in_order() {
    let items = ["a", "b", "c", "d", "e"];
    let log: Mutex<Vec<String>> = Mutex::new(vec![]);
    let started = AtomicUsize::new(0);
    let prepare = |name: &&'static str| -> Result<&'static str, String> {
        started.fetch_add(1, Ordering::SeqCst);
        log.lock().unwrap().push(format!("prep {name}"));
        Ok(*name)
    };
    let consume = |name: &&'static str, r: Result<&'static str, String>| {
        assert_eq!(r, Ok(*name), "{name}");
        let i = items.iter().position(|n| n == name).unwrap();
        if i + 1 < items.len() {
            // the next item's preparation starts while this one is consumed (the overlap)
            assert!(
                wait_until(5_000, || started.load(Ordering::SeqCst) >= i + 2),
                "{name}: the preparation of the next item did not start"
            );
        }
        log.lock().unwrap().push(format!("consume {name}"));
    };
    let n = run(&items, prepare, consume, || false);
    assert_eq!(n, items.len());
    let log = log.into_inner().unwrap();
    let consumed: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("consume ")).collect();
    assert_eq!(consumed, items, "{log:?}");
    for name in items {
        let p = log.iter().position(|l| l == &format!("prep {name}")).unwrap();
        let c = log.iter().position(|l| l == &format!("consume {name}")).unwrap();
        assert!(p < c, "{name}: prepared before consumed, {log:?}");
    }
    // the preparation runs on the pool, the consumption on the caller's thread
    let thread = Mutex::new(vec![]);
    run(
        &["x"],
        |_| -> Result<String, ()> { Ok(std::thread::current().name().unwrap_or("?").to_string()) },
        |_, r| thread.lock().unwrap().push((r.unwrap(), std::thread::current().id())),
        || false,
    );
    let t = thread.into_inner().unwrap();
    assert!(t[0].0.starts_with("ds-prepare-"), "{:?}", t[0]);
    assert_eq!(t[0].1, std::thread::current().id());
}

#[test]
fn at_most_two_prepared_items_are_alive() {
    let items: Vec<usize> = (0..12).collect();
    let counter = Counter::default();
    let prepare = |_: &usize| -> Result<Live<'_>, ()> {
        // fast preparation ...
        std::thread::sleep(Duration::from_millis(1));
        Ok(counter.make("item"))
    };
    let consume = |_: &usize, r: Result<Live<'_>, ()>| {
        // ... and a slow model step: an unbounded producer would run ahead
        let live = r.unwrap();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(live.name, "item");
        drop(live);
    };
    let n = run(&items, prepare, consume, || false);
    assert_eq!(n, items.len());
    assert_eq!(counter.live.load(Ordering::SeqCst), 0, "everything is dropped");
    let max = counter.max.load(Ordering::SeqCst);
    assert!(max <= 2, "at most 2 prepared items alive, saw {max}");
    assert_eq!(max, 2, "the overlap happened");
}

#[test]
fn a_cancel_during_the_overlap_stops_both_sides_after_the_current_item() {
    let items = ["a", "b", "c", "d", "e"];
    let cancel = AtomicBool::new(false);
    let prepared = Mutex::new(vec![]);
    let consumed = Mutex::new(vec![]);
    let prepare = |name: &&'static str| -> Result<&'static str, String> {
        prepared.lock().unwrap().push(*name);
        Ok(*name)
    };
    let consume = |name: &&'static str, r: Result<&'static str, String>| {
        assert!(r.is_ok());
        // the cancel arrives while "b" is in the model step; "c" is being prepared
        if *name == "b" {
            assert!(
                wait_until(5_000, || prepared.lock().unwrap().len() >= 3),
                "the preparation of c did not start during b"
            );
            cancel.store(true, Ordering::SeqCst);
        }
        consumed.lock().unwrap().push(*name);
    };
    let n = run(&items, prepare, consume, || cancel.load(Ordering::SeqCst));
    assert_eq!(n, 2);
    assert_eq!(*consumed.lock().unwrap(), ["a", "b"]);
    let p = prepared.into_inner().unwrap();
    assert_eq!(p, ["a", "b", "c"], "the producer stopped before d");
    // a cancel before the start: nothing runs
    let n = run(
        &items,
        |_| -> Result<(), ()> { panic!("prepared") },
        |_, _| panic!("consumed"),
        || true,
    );
    assert_eq!(n, 0);
}

#[test]
fn a_failing_item_is_handed_over_and_the_others_run() {
    let items = ["a", "b", "c", "d", "e"];
    let seen = Mutex::new(vec![]);
    let prepare = |name: &&'static str| -> Result<&'static str, String> {
        if *name == "c" {
            Err(format!("{name}: cannot decode"))
        } else {
            Ok(*name)
        }
    };
    let consume = |name: &&'static str, r: Result<&'static str, String>| {
        seen.lock().unwrap().push((*name, r));
    };
    let n = run(&items, prepare, consume, || false);
    assert_eq!(n, 5);
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.len(), 5);
    assert_eq!(seen[2], ("c", Err("c: cannot decode".to_string())));
    for (i, name) in items.iter().enumerate() {
        assert_eq!(seen[i].0, *name);
        if *name != "c" {
            assert_eq!(seen[i].1, Ok(*name), "{name}");
        }
    }
}
