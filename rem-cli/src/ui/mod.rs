// ── ui/mod.rs ── cli was written in rust so dont write in py same design but redesign the current cli
//
// Public surface of the UI layer: theme system, header banner, slash palette,
// prompt line, and streaming output. All terminal rendering flows through here.

pub mod header;
pub mod markdown;
pub mod output;
pub mod theme;
