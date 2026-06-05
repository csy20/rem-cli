// ── commands/mod.rs ── cli was written in rust so dont write in py same design but redesign the current cli
//
// Slash command registry: dataclass-style Command, the REGISTRY list, and
// shared state (conversation history) for handlers that need it.

pub mod registry;
