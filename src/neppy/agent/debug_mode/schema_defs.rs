//! Controller schema definitions for the `debug_mode` namespace (split from
//! `schemas.rs` to keep both files small). Pure data: handlers live next door.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub(super) fn field(
    name: &'static str,
    ty: TypeSchema,
    comment: &'static str,
    required: bool,
) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required,
    }
}

fn opt(ty: TypeSchema) -> TypeSchema {
    TypeSchema::Option(Box::new(ty))
}

fn out(comment: &'static str) -> Vec<FieldSchema> {
    vec![field("result", TypeSchema::Json, comment, true)]
}

fn root_field() -> FieldSchema {
    field(
        "project_root",
        opt(TypeSchema::String),
        "Project root (git work-tree root). Defaults to NEPPY_DEBUG_PROJECT_ROOT, then the \
         build-time source dir.",
        false,
    )
}

pub(super) fn schemas(function: &str) -> ControllerSchema {
    let (function, description, inputs, outputs): (&'static str, &'static str, _, _) =
        match function {
            "status" => (
                "status",
                "Project root, branch, HEAD, dirty files and whether a debug task is active.",
                vec![root_field()],
                out("DebugStatus."),
            ),
            "discover_checks" => (
                "discover_checks",
                "Validation checks derived from the project's package.json / Cargo.toml.",
                vec![root_field()],
                out("DebugCheck[]."),
            ),
            "checkpoint_create" => (
                "checkpoint_create",
                "Non-destructive snapshot of the working tree, pinned under refs/neppy-debug.",
                vec![
                    root_field(),
                    field(
                        "description",
                        TypeSchema::String,
                        "What this checkpoint marks.",
                        true,
                    ),
                    field(
                        "task_id",
                        opt(TypeSchema::String),
                        "Owning debug task.",
                        false,
                    ),
                ],
                out("Checkpoint."),
            ),
            "checkpoint_list" => (
                "checkpoint_list",
                "List checkpoints, newest first.",
                vec![field(
                    "limit",
                    opt(TypeSchema::U64),
                    "1..500 (default 50).",
                    false,
                )],
                out("Checkpoint[]."),
            ),
            "checkpoint_get" => (
                "checkpoint_get",
                "Get one checkpoint.",
                vec![field(
                    "checkpoint_id",
                    TypeSchema::String,
                    "Checkpoint id.",
                    true,
                )],
                out("Checkpoint."),
            ),
            "rollback" => (
                "rollback",
                "Restore tracked files from a checkpoint (after an automatic pre-rollback \
                 checkpoint) and remove files created since. Requires confirm=true.",
                vec![
                    root_field(),
                    field("checkpoint_id", TypeSchema::String, "Checkpoint id.", true),
                    field("confirm", TypeSchema::Bool, "Must be true.", true),
                ],
                out("RollbackResult."),
            ),
            "commit" => (
                "commit",
                "Commit exactly the files a debug task changed (git add of those paths, then \
                 git commit; hooks run, never pushes). Refuses if the index already has staged \
                 changes outside those paths. Requires confirm=true.",
                vec![
                    root_field(),
                    field("task_id", TypeSchema::String, "Debug task id.", true),
                    field("message", TypeSchema::String, "Commit message.", true),
                    field("confirm", TypeSchema::Bool, "Must be true.", true),
                ],
                out("CommitResult."),
            ),
            "diff" => (
                "diff",
                "Unified diff plus summary of the working tree vs HEAD or a checkpoint.",
                vec![
                    root_field(),
                    field(
                        "checkpoint_id",
                        opt(TypeSchema::String),
                        "Compare against this checkpoint.",
                        false,
                    ),
                ],
                out("DiffResult."),
            ),
            "task_start" => (
                "task_start",
                "Record a new debug task in status 'planning'.",
                vec![field(
                    "request",
                    TypeSchema::String,
                    "The user's request.",
                    true,
                )],
                out("TaskRecord."),
            ),
            "task_update" => (
                "task_update",
                "Update a debug task (status, files_changed, validation, summary, checkpoint_id, \
                 branch, commit).",
                vec![
                    field("task_id", TypeSchema::String, "Task id.", true),
                    field(
                        "status",
                        opt(TypeSchema::Enum {
                            variants: vec![
                                "planning",
                                "editing",
                                "validating",
                                "pass",
                                "partial",
                                "failed",
                                "rolled_back",
                            ],
                        }),
                        "New status.",
                        false,
                    ),
                    field("files_changed", opt(TypeSchema::Json), "string[].", false),
                    field(
                        "validation",
                        opt(TypeSchema::Json),
                        "ValidationRecord[] (replaces).",
                        false,
                    ),
                    field(
                        "summary",
                        opt(TypeSchema::String),
                        "Outcome summary.",
                        false,
                    ),
                    field(
                        "checkpoint_id",
                        opt(TypeSchema::String),
                        "Linked checkpoint.",
                        false,
                    ),
                    field("branch", opt(TypeSchema::String), "Git branch.", false),
                    field("commit", opt(TypeSchema::String), "Git commit.", false),
                ],
                out("TaskRecord."),
            ),
            "task_list" => (
                "task_list",
                "List debug tasks, newest first.",
                vec![field(
                    "limit",
                    opt(TypeSchema::U64),
                    "1..500 (default 50).",
                    false,
                )],
                out("TaskRecord[]."),
            ),
            "task_get" => (
                "task_get",
                "Get one debug task.",
                vec![field("task_id", TypeSchema::String, "Task id.", true)],
                out("TaskRecord."),
            ),
            "run_check" => (
                "run_check",
                "Run one discovered check (or an allowlisted argv) in the project root.",
                vec![
                    root_field(),
                    field(
                        "check_id",
                        opt(TypeSchema::String),
                        "Id from discover_checks.",
                        false,
                    ),
                    field(
                        "command",
                        opt(TypeSchema::Json),
                        "argv string[] (alternative to check_id).",
                        false,
                    ),
                    field(
                        "timeout_secs",
                        opt(TypeSchema::U64),
                        "Default 600, max 3600.",
                        false,
                    ),
                    field(
                        "task_id",
                        opt(TypeSchema::String),
                        "Record the result on this task.",
                        false,
                    ),
                ],
                out("CheckResult."),
            ),
            "audit_tail" => (
                "audit_tail",
                "Most recent audit-log entries (oldest first).",
                vec![field(
                    "limit",
                    opt(TypeSchema::U64),
                    "1..1000 (default 100).",
                    false,
                )],
                out("AuditEntry[]."),
            ),
            "settings_get" => (
                "settings_get",
                "The saved Debug Mode settings ([debug_mode] in config.toml).",
                vec![],
                out("DebugModeSettings."),
            ),
            "settings_update" => (
                "settings_update",
                "Partially update Debug Mode settings. Validates max_repair_iterations (1..=20), \
                 project_root (an existing git work-tree root) and external_paths (absolute, \
                 existing directories; never / , the home directory or a protected location).",
                vec![field(
                    "patch",
                    TypeSchema::Json,
                    "Object with any DebugModeSettings fields; project_root: null clears it.",
                    true,
                )],
                out("DebugModeSettings (the new settings)."),
            ),
            _ => (
                "unknown",
                "Unknown debug_mode controller function.",
                vec![field(
                    "function",
                    TypeSchema::String,
                    "Unknown function.",
                    true,
                )],
                vec![field(
                    "error",
                    TypeSchema::String,
                    "Lookup error details.",
                    true,
                )],
            ),
        };
    ControllerSchema {
        namespace: "debug_mode",
        function,
        description,
        inputs,
        outputs,
    }
}
