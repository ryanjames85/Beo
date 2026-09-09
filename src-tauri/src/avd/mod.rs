mod lifecycle;
mod naming;
mod profiles;
mod rotation;
mod snapshots;

// Glob re-exports, not named ones: #[tauri::command] leaves hidden helper
// items (`__cmd__*`) alongside each function in its defining submodule, and
// `generate_handler!` in lib.rs looks those up at the same path it's given
// for the function itself (`avd::list_avds`) — a named `pub use` only
// re-exports the function, not its hidden companion, and fails to compile.
pub(crate) use lifecycle::*;
pub(crate) use profiles::*;
pub(crate) use rotation::*;
pub(crate) use snapshots::*;
