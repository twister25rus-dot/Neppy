//! Route families served by the local backend.
//!
//! Each module owns one family and is responsible for matching the hosted
//! response *shape* — field names, nesting, status codes — because the clients
//! parsing them are the hosted clients, unchanged. Where a hosted field has no
//! local meaning it is filled with the value that reads correctly rather than
//! omitted: `credits: null` (there is no balance), not a missing key that makes
//! a strict parser throw.

pub(crate) mod auth;
pub(crate) mod inference;
pub(crate) mod meta;
pub(crate) mod teams;
pub(crate) mod unsupported;
