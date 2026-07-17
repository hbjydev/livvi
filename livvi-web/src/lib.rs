//! Web tools for Livvi: search and fetch pages from the public internet.

pub mod tools;

mod plugin;
mod state;

pub use plugin::WebPlugin;
pub use state::WebState;
