//! Product-independent payment adapters. No invoice, booking, or shop authority lives here.
#[cfg(test)]
mod acceptance_tests;
pub(crate) mod browser;
pub mod checkout;
pub mod delivery;
pub mod http;
pub mod operation;
pub mod repository;
pub mod requests;
#[cfg(test)]
mod sandbox_tests;
pub mod stripe;
#[cfg(test)]
mod typedb_tests;
pub mod webhook;
