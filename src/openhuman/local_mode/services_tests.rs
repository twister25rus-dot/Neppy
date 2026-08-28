//! Tests for the hosted-service inventory.
//!
//! These are contract tests, not coverage padding: the inventory is consumed
//! by the local backend's error responses, by the Settings UI over RPC, and by
//! the generated `docs/LOCAL_MODE.md`. A malformed entry ships a broken error
//! message to an agent.

use super::*;
use std::collections::HashSet;

#[test]
fn every_entry_has_a_unique_stable_id() {
    let inventory = service_inventory();
    let mut seen = HashSet::new();
    for entry in &inventory.entries {
        assert!(
            seen.insert(entry.id),
            "duplicate service id `{}` — ids are the join key for the UI and docs",
            entry.id
        );
        assert!(
            entry.id.contains('.'),
            "`{}` should be a dotted `domain.capability` id",
            entry.id
        );
    }
}

#[test]
fn every_entry_explains_its_local_alternative() {
    for entry in &service_inventory().entries {
        assert!(
            !entry.hosted.trim().is_empty(),
            "`{}` has no hosted description",
            entry.id
        );
        assert!(
            entry.local_alternative.trim().len() > 20,
            "`{}` needs a real alternative, not a placeholder",
            entry.id
        );
    }
}

#[test]
fn setup_text_is_present_exactly_when_setup_is_required() {
    for entry in &service_inventory().entries {
        match entry.kind {
            LocalReplacementKind::RequiresSetup => assert!(
                !entry.setup.trim().is_empty(),
                "`{}` is RequiresSetup but tells the user nothing to do",
                entry.id
            ),
            _ => assert!(
                entry.setup.trim().is_empty(),
                "`{}` is {:?} yet carries setup instructions — the UI would show a \
                 next action for something that needs none",
                entry.id,
                entry.kind
            ),
        }
    }
}

#[test]
fn counts_partition_the_entries() {
    let inventory = service_inventory();
    assert_eq!(
        inventory.replaced
            + inventory.requires_setup
            + inventory.unavailable
            + inventory.not_applicable,
        inventory.entries.len(),
        "the headline counts must add up to the inventory"
    );
}

#[test]
fn availability_follows_the_kind() {
    assert!(LocalReplacementKind::Replaced.is_available());
    // RequiresSetup keeps the surface: the UI prompts for the missing piece
    // instead of hiding the feature.
    assert!(LocalReplacementKind::RequiresSetup.is_available());
    assert!(!LocalReplacementKind::Unavailable.is_available());
    assert!(!LocalReplacementKind::NotApplicable.is_available());
}

#[test]
fn routes_are_absolute_and_unslashed() {
    for entry in &service_inventory().entries {
        for route in entry.routes {
            assert!(
                route.starts_with('/'),
                "`{}` route `{route}` must be absolute",
                entry.id
            );
            assert!(
                !route.ends_with('/'),
                "`{}` route `{route}` must not carry a trailing slash — \
                 prefix matching adds the boundary itself",
                entry.id
            );
        }
    }
}

#[test]
fn a_route_is_claimed_by_exactly_one_entry() {
    let inventory = service_inventory();
    let mut seen = HashSet::new();
    for entry in &inventory.entries {
        for route in entry.routes {
            assert!(
                seen.insert(*route),
                "route `{route}` is claimed twice — the local backend would pick \
                 whichever entry sorts first"
            );
        }
    }
}

#[test]
fn path_lookup_finds_the_owning_entry() {
    assert_eq!(
        entry_for_path("/teams/me/usage").map(|e| e.id),
        Some("account.usage")
    );
    assert_eq!(
        entry_for_path("/payments/stripe/currentPlan").map(|e| e.id),
        Some("account.billing")
    );
    assert_eq!(
        entry_for_path("/auth/me").map(|e| e.id),
        Some("auth.session")
    );
}

#[test]
fn path_lookup_prefers_the_longest_matching_route() {
    // `/agent-integrations/composio/execute` is claimed by the composio entry,
    // not by any shorter `/agent-integrations` prefix — otherwise the error
    // would recommend the wrong alternative.
    assert_eq!(
        entry_for_path("/agent-integrations/composio/execute").map(|e| e.id),
        Some("integrations.composio")
    );
    assert_eq!(
        entry_for_path("/agent-integrations/parallel/research").map(|e| e.id),
        Some("integrations.research")
    );
}

#[test]
fn path_lookup_requires_a_segment_boundary() {
    // `/searchable` is not `/search`: without the boundary check the user
    // would be told to install SearXNG for an unrelated route.
    assert!(entry_for_path("/searchable").is_none());
    assert!(entry_for_path("/teamsomething").is_none());
    // The boundary may be a query string as well as a slash.
    assert_eq!(
        entry_for_path("/search?q=x").map(|e| e.id),
        Some("search.web")
    );
}

#[test]
fn path_lookup_returns_none_for_an_unclaimed_route() {
    assert!(entry_for_path("/some/route/no/entry/claims").is_none());
}

#[test]
fn entries_without_routes_are_informational_only() {
    // `memory.storage` documents that nothing changed; it owns no backend
    // route and must never be selected as an error's alternative.
    let memory = service_inventory()
        .entries
        .into_iter()
        .find(|e| e.id == "memory.storage")
        .expect("memory.storage entry");
    assert!(memory.routes.is_empty());
    assert_eq!(memory.kind, LocalReplacementKind::Replaced);
}

#[test]
fn third_party_saas_is_recorded_as_unavailable_not_replaced() {
    // The honesty guarantee: anything we cannot legally or technically run
    // locally must never claim to be Replaced. A fabricated success here is
    // how an agent ends up acting on invented search results.
    let inventory = service_inventory();
    for id in [
        "integrations.composio",
        "integrations.research",
        "media.generation",
    ] {
        let entry = inventory
            .entries
            .iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| panic!("{id} entry"));
        assert_eq!(
            entry.kind,
            LocalReplacementKind::Unavailable,
            "{id} wraps a third-party service and cannot be Replaced"
        );
    }
}

#[test]
fn kind_round_trips_as_snake_case_json() {
    // The RPC contract the Settings UI reads.
    for (kind, json) in [
        (LocalReplacementKind::Replaced, "\"replaced\""),
        (LocalReplacementKind::RequiresSetup, "\"requires_setup\""),
        (LocalReplacementKind::Unavailable, "\"unavailable\""),
        (LocalReplacementKind::NotApplicable, "\"not_applicable\""),
    ] {
        assert_eq!(serde_json::to_string(&kind).unwrap(), json);
        assert_eq!(
            serde_json::from_str::<LocalReplacementKind>(json).unwrap(),
            kind
        );
    }
}

#[test]
fn every_entry_is_documented_in_local_mode_md() {
    // The drift guard named in the module docs. Adding a service entry without
    // a row in the user-facing table is the failure this catches: the UI would
    // show a capability the documentation never explains.
    let doc = include_str!("../../../docs/LOCAL_MODE.md");
    for entry in &service_inventory().entries {
        assert!(
            doc.contains(entry.id),
            "`{}` is in the inventory but not in docs/LOCAL_MODE.md",
            entry.id
        );
    }
}

#[test]
fn the_documented_sections_match_the_kinds_in_use() {
    // A weaker but useful companion to the id check: every kind that any entry
    // actually uses must have a section heading, so a new kind cannot be added
    // to the enum and shipped with nowhere to describe it.
    let doc = include_str!("../../../docs/LOCAL_MODE.md");
    let inventory = service_inventory();
    let heading_for = |kind: LocalReplacementKind| match kind {
        LocalReplacementKind::Replaced => "### Replaced",
        LocalReplacementKind::RequiresSetup => "### Needs setup",
        LocalReplacementKind::Unavailable => "### No local equivalent",
        LocalReplacementKind::NotApplicable => "### Not applicable",
    };
    for entry in &inventory.entries {
        let heading = heading_for(entry.kind);
        assert!(
            doc.contains(heading),
            "docs/LOCAL_MODE.md has no `{heading}` section, but `{}` is {:?}",
            entry.id,
            entry.kind
        );
    }
}
