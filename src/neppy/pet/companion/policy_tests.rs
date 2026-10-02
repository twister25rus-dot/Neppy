use super::*;
use crate::neppy::pet::companion::types::{ActionCategory as C, CompanionLevel as L};

const TIERS: [AutonomyLevel; 3] = [
    AutonomyLevel::ReadOnly,
    AutonomyLevel::Supervised,
    AutonomyLevel::Full,
];

/// Settings with every category at `cat_level`, bypassing patch validation:
/// high-risk categories are deliberately set to Trusted to prove the policy
/// clamps them itself even if storage were corrupt.
fn settings(level: L, cat_level: L) -> CompanionSettings {
    let mut s = CompanionSettings::default();
    s.level = level;
    for c in C::ALL {
        s.category_levels.insert(*c, cat_level);
    }
    s
}

fn ctx(tier: AutonomyLevel, approve_all: bool, user: bool) -> PolicyCtx {
    PolicyCtx {
        tier,
        auto_approve_all: approve_all,
        initiated_by_user: user,
    }
}

#[test]
fn tier_caps() {
    assert_eq!(tier_cap(AutonomyLevel::ReadOnly), L::Suggest);
    assert_eq!(tier_cap(AutonomyLevel::Supervised), L::Assist);
    assert_eq!(tier_cap(AutonomyLevel::Full), L::Trusted);
}

#[test]
fn effective_level_is_the_minimum_of_the_three() {
    let mut s = settings(L::Trusted, L::Trusted);
    s.category_levels.insert(C::Explain, L::Assist);
    assert_eq!(
        effective_level(&s, C::Explain, AutonomyLevel::Full),
        L::Assist
    );
    assert_eq!(
        effective_level(&s, C::DraftText, AutonomyLevel::Full),
        L::Trusted
    );
    assert_eq!(
        effective_level(&s, C::DraftText, AutonomyLevel::Supervised),
        L::Assist
    );
    assert_eq!(
        effective_level(&s, C::DraftText, AutonomyLevel::ReadOnly),
        L::Suggest
    );
    let s2 = settings(L::Observe, L::Trusted);
    assert_eq!(
        effective_level(&s2, C::Explain, AutonomyLevel::Full),
        L::Observe
    );
}

#[test]
fn high_risk_is_pinned_at_suggest_even_with_everything_maxed() {
    let s = settings(L::Trusted, L::Trusted);
    for c in C::HIGH_RISK {
        assert!(
            effective_level(&s, *c, AutonomyLevel::Full) <= L::Suggest,
            "{c:?}"
        );
    }
}

#[test]
fn high_risk_is_always_refused_across_the_whole_matrix() {
    for level in L::ALL {
        for cat_level in L::ALL {
            let s = settings(*level, *cat_level);
            for tier in TIERS {
                for approve_all in [false, true] {
                    for user in [false, true] {
                        for c in C::HIGH_RISK {
                            let d = decide(&s, *c, &ctx(tier, approve_all, user));
                            assert_eq!(
                                d,
                                Decision::Refuse,
                                "{c:?} level={level:?} tier={tier:?} approve_all={approve_all} user={user}"
                            );
                            assert!(!d.is_autonomous());
                            assert_eq!(d.audit(), ActionDecision::RefusedHighRisk);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn full_matrix_invariants_for_every_category() {
    for level in L::ALL {
        for cat_level in L::ALL {
            let s = settings(*level, *cat_level);
            for tier in TIERS {
                for approve_all in [false, true] {
                    for user in [false, true] {
                        for c in C::ALL {
                            let eff = effective_level(&s, *c, tier);
                            let d = decide(&s, *c, &ctx(tier, approve_all, user));
                            let label = format!(
                                "{c:?} level={level:?}/{cat_level:?} tier={tier:?} all={approve_all} user={user} -> {d:?}"
                            );
                            if c.is_high_risk() {
                                assert_eq!(d, Decision::Refuse, "{label}");
                                continue;
                            }
                            assert_ne!(d, Decision::Refuse, "{label}");
                            // Autonomy only from Assist up, and only for the
                            // auto-capable categories.
                            if d == Decision::ExecuteAuto {
                                assert!(eff >= L::Assist, "{label}");
                                assert!(
                                    matches!(
                                        c,
                                        C::Explain | C::DraftText | C::SaveNote | C::HandoffTask
                                    ),
                                    "{label}"
                                );
                            }
                            // Never more than the effective level allows.
                            if eff == L::Observe {
                                assert_eq!(
                                    d,
                                    if user {
                                        Decision::Suggest
                                    } else {
                                        Decision::Drop
                                    },
                                    "{label}"
                                );
                            }
                            if eff == L::Suggest {
                                assert_eq!(
                                    d,
                                    if user {
                                        Decision::ExecuteOnConfirm
                                    } else {
                                        Decision::Suggest
                                    },
                                    "{label}"
                                );
                            }
                            // Proactive decisions are never Execute* below Assist.
                            if !user && eff < L::Assist {
                                assert!(
                                    !matches!(
                                        d,
                                        Decision::ExecuteAuto | Decision::ExecuteOnConfirm
                                    ),
                                    "{label}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn approve_everything_never_unlocks_high_risk_but_does_unlock_handoff() {
    let s = settings(L::Trusted, L::Trusted);
    let full_all = ctx(AutonomyLevel::Full, true, false);
    for c in C::HIGH_RISK {
        assert_eq!(decide(&s, *c, &full_all), Decision::Refuse);
    }
    // Handoff at Assist: needs approve-everything (or Trusted).
    let assist = settings(L::Assist, L::Assist);
    assert_eq!(
        decide(
            &assist,
            C::HandoffTask,
            &ctx(AutonomyLevel::Full, false, false)
        ),
        Decision::Suggest
    );
    assert_eq!(
        decide(
            &assist,
            C::HandoffTask,
            &ctx(AutonomyLevel::Full, true, false)
        ),
        Decision::ExecuteAuto
    );
    assert_eq!(
        decide(&s, C::HandoffTask, &ctx(AutonomyLevel::Full, false, false)),
        Decision::ExecuteAuto
    );
    // The tier still caps: a Supervised tier cannot reach Trusted.
    assert_eq!(
        decide(
            &s,
            C::HandoffTask,
            &ctx(AutonomyLevel::Supervised, false, false)
        ),
        Decision::Suggest
    );
}

#[test]
fn defaults_behave_as_documented() {
    let s = CompanionSettings::default();
    let proactive = ctx(AutonomyLevel::Supervised, false, false);
    let click = ctx(AutonomyLevel::Supervised, false, true);
    // Default level Suggest: nothing runs without a click.
    assert_eq!(decide(&s, C::Explain, &proactive), Decision::Suggest);
    assert_eq!(decide(&s, C::Explain, &click), Decision::ExecuteOnConfirm);
    // Assist runs explain/draft/save automatically, not format/command/chat.
    let mut a = s.clone();
    a.level = L::Assist;
    for c in [C::Explain, C::DraftText, C::SaveNote] {
        assert_eq!(decide(&a, c, &proactive), Decision::ExecuteAuto, "{c:?}");
    }
    for c in [C::FormatText, C::PrepareCommand, C::OpenChat] {
        assert_ne!(decide(&a, c, &proactive), Decision::ExecuteAuto, "{c:?}");
    }
    // Read-only tier keeps even Assist at Suggest.
    assert_eq!(
        decide(&a, C::Explain, &ctx(AutonomyLevel::ReadOnly, false, false)),
        Decision::Suggest
    );
}

#[test]
fn observe_drops_proactive_but_lets_the_user_ask() {
    let mut s = CompanionSettings::default();
    s.level = L::Observe;
    assert_eq!(
        decide(&s, C::Explain, &ctx(AutonomyLevel::Full, true, false)),
        Decision::Drop
    );
    assert_eq!(
        decide(&s, C::Explain, &ctx(AutonomyLevel::Full, true, true)),
        Decision::Suggest
    );
}

#[test]
fn per_category_levels_are_independent() {
    let mut s = settings(L::Trusted, L::Assist);
    s.category_levels.insert(C::SaveNote, L::Observe);
    let c = ctx(AutonomyLevel::Full, false, false);
    assert_eq!(decide(&s, C::SaveNote, &c), Decision::Drop);
    assert_eq!(decide(&s, C::Explain, &c), Decision::ExecuteAuto);
}

#[test]
fn audit_mapping() {
    assert_eq!(Decision::ExecuteAuto.audit(), ActionDecision::Auto);
    assert_eq!(
        Decision::ExecuteOnConfirm.audit(),
        ActionDecision::Confirmed
    );
    assert_eq!(Decision::Suggest.audit(), ActionDecision::BlockedPolicy);
    assert_eq!(Decision::Drop.audit(), ActionDecision::BlockedPolicy);
}
