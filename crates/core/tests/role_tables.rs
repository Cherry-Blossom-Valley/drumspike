//! The role tables pin the MIDI notes a detected hit gets and the role names the stored
//! files carry: the detector note per role, the note per detector class, the notes a role
//! keeps, the tempo voice of a role name and the serde form of a role. A change in any of
//! them changes the exported MIDI bytes or the read of a stored candidates file.

use drumspike_core::analyze::{role_classes, role_note};
use drumspike_core::backend::class_note;
use drumspike_core::detect::CLASS_NOTES;
use drumspike_core::detect::onset::{OnsetParams, Role};

/// The role names of the app plus an unknown name and the empty name.
const NAMES: [&str; 8] = ["kick", "snare", "tom", "hat", "cymbal", "other", "bass", ""];

#[test]
fn the_detector_note_of_each_role_name_is_fixed_and_unknown_names_take_the_kick_note() {
    let notes: Vec<u8> = NAMES.iter().map(|r| role_note(r)).collect();
    assert_eq!(notes, [35, 38, 47, 42, 49, 35, 35, 35]);
    // the case matters: only the lower-case app names are roles
    assert_eq!(role_note("Snare"), 35);
}

#[test]
fn the_class_notes_are_the_five_detector_notes() {
    assert_eq!(CLASS_NOTES, [35, 38, 47, 42, 49]);
}

#[test]
fn a_class_index_maps_to_its_class_note_and_the_other_class_to_the_role_note() {
    for role in NAMES {
        let got: Vec<u8> = (0..=7u8).map(|c| class_note(c, role)).collect();
        let r = role_note(role);
        assert_eq!(got, [35, 38, 47, 42, 49, r, r, r], "{role}");
    }
    assert_eq!(class_note(u8::MAX, "snare"), 38);
}

#[test]
fn each_role_keeps_its_own_notes_and_unknown_names_keep_every_drum_note() {
    let other: Vec<u8> = (28..60).collect();
    let want: [&[u8]; 8] = [
        &[35],
        &[38],
        &[41, 43, 45, 47, 48, 50],
        &[42, 44, 46],
        &[49, 51, 52, 53, 55],
        &other,
        &other,
        &other,
    ];
    for (role, want) in NAMES.iter().zip(want) {
        assert_eq!(role_classes(role), want, "{role}");
    }
}

#[test]
fn a_role_is_stored_by_its_variant_name_and_a_missing_role_reads_as_other() {
    let names: Vec<String> = Role::ALL
        .iter()
        .map(|r| serde_json::to_value(r).unwrap().as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["Kick", "Snare", "Tom", "Hat", "Cymbal", "Other"]);
    for r in Role::ALL {
        let v = serde_json::to_value(r).unwrap();
        assert_eq!(serde_json::from_value::<Role>(v).unwrap(), r);
    }
    assert!(serde_json::from_value::<Role>(serde_json::json!("kick")).is_err());
    assert_eq!(Role::default(), Role::Other);

    let p = OnsetParams::for_role(Role::Kick);
    let mut v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["role"], "Kick");
    v.as_object_mut().unwrap().remove("role");
    let old: OnsetParams = serde_json::from_value(v).unwrap();
    assert_eq!(old.role, Role::Other);
}

#[test]
fn the_app_role_names_parse_in_the_app_order() {
    let names: Vec<&str> = Role::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(names, ["kick", "snare", "tom", "hat", "cymbal", "other"]);
    for (i, r) in Role::ALL.iter().enumerate() {
        assert_eq!(Role::parse(NAMES[i]), Some(*r));
    }
    assert_eq!(Role::parse("bass"), None);
    assert_eq!(Role::parse(""), None);
    assert_eq!(Role::parse("Kick"), None);
    let edge: Vec<Role> = NAMES.iter().map(|n| Role::parse_or_other(n)).collect();
    assert_eq!(edge[..6], Role::ALL);
    assert_eq!(edge[6..], [Role::Other, Role::Other]);
}
