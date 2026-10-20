//! Public API acceptance tests for frontend TypeQL, authentication and tenant policy.
use server::app::{build_with_state, router};
use server::database::*;

mod domain_model_tests;
mod provisioning_tests;
mod review_tests;
mod schema_policy_tests;
