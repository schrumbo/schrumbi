pub mod cmd;
pub mod locate;
pub mod status;
pub mod sync;
#[cfg(test)]
mod tests;

pub use cmd::{Git, GitError};
