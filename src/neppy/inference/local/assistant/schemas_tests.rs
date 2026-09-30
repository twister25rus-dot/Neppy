use super::*;

#[test]
fn every_function_has_a_schema_and_a_handler() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(schemas.len(), FUNCTIONS.len());
    assert_eq!(controllers.len(), FUNCTIONS.len());
    for (schema, controller) in schemas.iter().zip(controllers.iter()) {
        assert_eq!(schema.namespace, "local_assistant");
        assert_eq!(schema.function, controller.schema.function);
        assert!(!schema.description.is_empty());
    }
    let names: Vec<_> = schemas.iter().map(|s| s.function).collect();
    for expected in [
        "start_task",
        "status",
        "list",
        "resume",
        "cancel",
        "set_enabled",
        "index_refresh",
        "index_status",
        "search",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
    }
}

#[test]
fn start_task_marks_only_root_and_goal_required() {
    let schema = schemas("start_task");
    let required: Vec<_> = schema
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["project_root", "goal"]);
}

#[test]
fn params_reject_unknown_shapes_with_a_readable_error() {
    let mut params = Map::new();
    params.insert("goal".into(), Value::String("g".into()));
    let err = parse::<StartParams>(params).err().unwrap();
    assert!(err.starts_with("invalid params:"), "{err}");
    let mut ok = Map::new();
    ok.insert("project_root".into(), Value::String("/p".into()));
    ok.insert("goal".into(), Value::String("g".into()));
    let parsed = parse::<StartParams>(ok).unwrap();
    assert!(!parsed.allow_edits, "edits are opt-in");
    assert!(parsed.test_command.is_none());
}
