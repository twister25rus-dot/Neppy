//! Tests for the `mlx` RPC surface.
//!
//! These pin the contract the UI codes against: the namespace, the function
//! set, and that every advertised function is actually registered with a
//! handler. A schema without a handler is an unknown-method at runtime, which
//! is exactly the failure the registry exists to prevent.

use super::*;

#[test]
fn every_advertised_function_has_a_handler() {
    let advertised: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|schema| schema.function)
        .collect();
    let registered: Vec<&str> = all_registered_controllers()
        .iter()
        .map(|controller| controller.schema.function)
        .collect();

    assert_eq!(
        advertised, registered,
        "schemas and registered controllers must stay in lockstep"
    );
}

#[test]
fn the_namespace_is_mlx_everywhere() {
    for schema in all_controller_schemas() {
        assert_eq!(
            schema.namespace, "mlx",
            "`{}` is in the wrong namespace",
            schema.function
        );
    }
}

#[test]
fn the_surface_covers_the_lifecycle_the_panel_needs() {
    let functions: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|schema| schema.function)
        .collect();

    for expected in ["status", "start", "stop", "restart", "logs", "unload"] {
        assert!(
            functions.contains(&expected),
            "`mlx.{expected}` is missing from the surface"
        );
    }
}

#[test]
fn lifecycle_functions_require_a_server_id() {
    // Every per-server call addresses a block by id; omitting it would make
    // the call ambiguous once more than one block is configured.
    for function in ["start", "stop", "restart", "logs", "unload"] {
        let schema = schemas(function);
        let id = schema
            .inputs
            .iter()
            .find(|field| field.name == "id")
            .unwrap_or_else(|| panic!("`mlx.{function}` has no id input"));
        assert!(id.required, "`mlx.{function}`'s id must be required");
    }
}

#[test]
fn status_takes_no_inputs() {
    // The panel polls this; requiring params would make an idle poll awkward
    // for no benefit.
    assert!(schemas("status").inputs.is_empty());
}

#[test]
fn logs_limit_is_optional() {
    let schema = schemas("logs");
    let limit = schema
        .inputs
        .iter()
        .find(|field| field.name == "limit")
        .expect("logs takes a limit");
    assert!(!limit.required);
}

#[test]
#[should_panic(expected = "unknown mlx controller function")]
fn an_unknown_function_panics_rather_than_returning_a_wrong_schema() {
    // Loud beats a silently mismatched schema: this is construction-time,
    // reached only by a programming error in the registration list.
    let _ = schemas("not_a_real_function");
}
