pub mod cmd;
pub mod conflict;
pub mod history;
pub mod locate;
pub mod merge_json;
pub mod message;
pub mod setup;
pub mod status;
pub mod sync;
#[cfg(test)]
mod tests;

pub use cmd::{Git, GitError};
