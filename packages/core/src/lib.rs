//! Core of the _k0t1k launcher: everything that does not depend on the UI shell.

pub mod download;
pub mod error;
pub mod paths;
pub mod progress;

pub use error::{Error, Result};
