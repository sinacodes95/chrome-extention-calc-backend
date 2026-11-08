//! Scientific calculations and commercial property investment modeling.

mod api;
pub mod finance;
pub mod property;
pub mod scientific;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod wasm;

use std::fmt;

pub use api::{calculate_json, MAX_REQUEST_BYTES};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalcError {
    InvalidInput(String),
    Domain(String),
    Numerical(String),
    NoSolution(String),
}

impl fmt::Display for CalcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidInput(message)
            | Self::Domain(message)
            | Self::Numerical(message)
            | Self::NoSolution(message) => message,
        };
        f.write_str(message)
    }
}

impl std::error::Error for CalcError {}
