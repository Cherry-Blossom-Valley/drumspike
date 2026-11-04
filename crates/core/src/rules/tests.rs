use super::*;

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let pairs: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    move |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
}

/// #281 A5: the delta-rule pin keeps the old parse: unset and unparseable are version 1, 0 pins
/// the absolute delta, every version from 1 up is the gained rule.
#[test]
fn the_delta_rule_pin_parses_like_the_old_env_reader() {
    assert_eq!(DeltaRule::from_pin(None), DeltaRule::Gained);
    assert_eq!(DeltaRule::from_pin(Some("x")), DeltaRule::Gained);
    assert_eq!(DeltaRule::from_pin(Some("0")), DeltaRule::Absolute);
    assert_eq!(DeltaRule::from_pin(Some("1")), DeltaRule::Gained);
    assert_eq!(DeltaRule::from_pin(Some("7")), DeltaRule::Gained);
    assert_eq!(
        DeltaRule::from_pin(Some("-1")),
        DeltaRule::Gained,
        "not a u32: version 1"
    );
    assert_eq!(DeltaRule::default(), DeltaRule::CURRENT);
    for v in [0, 1] {
        assert_eq!(DeltaRule::from_version(v).version(), v);
    }
}

/// #130, #180: the tempo-referee pin keeps the old parse: unset and unparseable are version 3,
/// 0 and 1 pin the shipped referee, 2 the #130 one, every larger number the #180 one.
#[test]
fn the_tempo_rule_pin_parses_like_the_old_env_reader() {
    assert_eq!(TempoRule::from_pin(None), TempoRule::HalfTime);
    assert_eq!(TempoRule::from_pin(Some("x")), TempoRule::HalfTime);
    assert_eq!(TempoRule::from_pin(Some("0")), TempoRule::Shipped);
    assert_eq!(TempoRule::from_pin(Some("1")), TempoRule::Shipped);
    assert_eq!(TempoRule::from_pin(Some("2")), TempoRule::Corroborated);
    assert_eq!(TempoRule::from_pin(Some("3")), TempoRule::HalfTime);
    assert_eq!(TempoRule::from_pin(Some("9")), TempoRule::HalfTime);
    assert_eq!(TempoRule::default(), TempoRule::CURRENT);
    for v in [1, 2, 3] {
        assert_eq!(TempoRule::from_version(v).version(), v);
    }
}

#[test]
fn a_rule_set_reads_its_two_pins_by_their_variable_names() {
    assert_eq!(RuleSet::from_lookup(lookup(&[])), RuleSet::default());
    let r = RuleSet::from_lookup(lookup(&[("DRUMSPIKE_DELTA_RULE", "0"), ("DRUMSPIKE_TEMPO_RULE", "2")]));
    assert_eq!(
        r,
        RuleSet {
            delta: DeltaRule::Absolute,
            tempo: TempoRule::Corroborated
        }
    );
}

/// Every research variable lands in its own field; an unparseable value overrides nothing.
#[test]
fn research_overrides_read_every_variable_by_name() {
    assert_eq!(
        ResearchOverrides::from_lookup(lookup(&[])),
        ResearchOverrides::default()
    );
    let o = ResearchOverrides::from_lookup(lookup(&[
        ("DRUMSPIKE_A1_DELTA", "1.5"),
        ("DRUMSPIKE_A1_SOFT_SIM", "0"),
        ("DRUMSPIKE_A1_SOFT_DB", "25"),
        ("DRUMSPIKE_A1_SOFT_T2", "0.8"),
        ("DRUMSPIKE_A1_SOFT_ATTACK", "19.5"),
        ("DRUMSPIKE_A1_OH_MARGIN_DB", "3"),
        ("DRUMSPIKE_A5_GAIN_ATTACK", "0"),
        ("DRUMSPIKE_A5_QUIET_GAIN", "1.5"),
        ("DRUMSPIKE_DELTA_REF_KICK", "20"),
        ("DRUMSPIKE_DELTA_REF_SNARE", "21"),
        ("DRUMSPIKE_DELTA_REF_HAT", "1.2"),
        ("DRUMSPIKE_DELTA_REF_TOM", "4"),
        ("DRUMSPIKE_DELTA_REF_CYMBAL", "2"),
        ("DRUMSPIKE_DELTA_REF_OTHER", "3"),
        ("DRUMSPIKE_A6_MIN_LEAD", "0.5"),
        ("DRUMSPIKE_A6_MARGIN_DB", "1"),
        ("DRUMSPIKE_A6_ALIGN_MIN_COINS", "12.7"),
        ("DRUMSPIKE_A6_ALIGN_MED_ABS_LEAD", "x"),
    ]));
    assert_eq!(o.a1_delta, Some(1.5));
    assert_eq!(o.a1_soft_sim, Some(0.0));
    assert_eq!(o.a1_soft_db, Some(25.0));
    assert_eq!(o.a1_soft_t2, Some(0.8));
    assert_eq!(o.a1_soft_attack, Some(19.5));
    assert_eq!(o.a1_oh_margin_db, Some(3.0));
    assert_eq!(o.a5_gain_attack, Some(0.0));
    assert_eq!(o.a5_quiet_gain, Some(1.5));
    let roles = [Role::Kick, Role::Snare, Role::Hat, Role::Tom, Role::Cymbal, Role::Other];
    let refs: Vec<Option<f32>> = roles.iter().map(|&r| o.delta_ref.of(r)).collect();
    assert_eq!(
        refs,
        [Some(20.0), Some(21.0), Some(1.2), Some(4.0), Some(2.0), Some(3.0)]
    );
    assert_eq!(o.a6_min_lead, Some(0.5));
    assert_eq!(o.a6_margin_db, Some(1.0));
    assert_eq!(o.a6_align_min_coins, Some(12.7));
    assert_eq!(o.a6_align_med_abs_lead, None, "unparseable: the shipped value stays");
}
