use super::*;

#[test]
fn window_has_a_fixed_length_reads_zero_outside_and_rounds_to_6_decimals() {
    let x = [0.123_456_7_f32, -0.5, 1.0];
    let w = window(&x, 1, 2, 3);
    assert_eq!(w.len(), 5);
    assert_eq!(
        w,
        vec![json!(0.0), json!(0.123_457), json!(-0.5), json!(1.0), json!(0.0)]
    );
    assert_eq!(window(&x, -10, 2, 2), vec![json!(0.0); 4]);
    assert_eq!(window(&[], 0, 1, 1).len(), 2);
}

#[test]
fn project_key_is_12_hex_digits_and_follows_the_seed() {
    let a = project_key(Some("1700000000.5".into()), "A");
    assert_eq!(a.len(), 12);
    assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(
        a,
        project_key(Some("1700000000.5".into()), "B"),
        "a rename keeps the key"
    );
    assert_ne!(a, project_key(None, "A"));
    assert_eq!(project_key(None, "A"), project_key(None, "A"));
}

#[test]
fn count_lines_counts_records_and_a_missing_file_has_none() {
    let p = std::env::temp_dir().join(format!("ds_training_count_{}.jsonl", std::process::id()));
    assert_eq!(count_lines(&p), 0);
    std::fs::write(&p, "{}\n{}\n").unwrap();
    assert_eq!(count_lines(&p), 2);
    let _ = std::fs::remove_file(&p);
}

#[test]
fn dispatch_needs_a_sid_and_knows_only_its_own_command() {
    let svc = Service::new(crate::ServiceConfig::default());
    assert!(dispatch(&svc, "events_get", &json!({})).is_none());
    assert_eq!(
        dispatch(&svc, "training_status", &json!({})).unwrap().unwrap_err().code,
        400
    );
    assert_eq!(TRAINING_COMMANDS, ["training_status"]);
}
