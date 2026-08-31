# TinyBus Adapter

This module is the boundary between the hosting library and TinyBus module ABI
v1. `HostingService` exposes two methods: `Execute`, which takes one JSON
hosting request and returns one JSON result, and `Providers`, which reports the
provider slugs this build was compiled with. `setup` registers the object and
claims the well-known interface name.

Both methods delegate straight into `crate::rpc`. `Execute` is one method rather
than one per operation because the JSON envelope is the contract either way —
the front end that calls this does not link against the crate — and fourteen bus
signatures would have to be kept in step with the Rust API, the envelope, and
the manifest at once.

`tinybus_module::module_export!` emits the descriptor, embedded manifest, and
initialization symbols consumed by the dynamic loader. The manifest method list
must stay aligned with the interface macro's dispatch table; the unit test checks
that relationship. Integration tests use TinyBus's in-memory transport against a
local mock of the provider API, and `examples/verify_module.rs` loads a compiled
`cdylib` through the real dynamic loader before a release archive is accepted.

This module is handed live hosting credentials by its callers. It must not log a
request, cache one, or retain Rust-owned data across the ABI boundary.
