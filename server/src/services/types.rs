use dog_auth::hooks::authenticate::AuthParams;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RestParams {
    #[serde(skip)]
    pub stripe_access:
        Option<std::sync::Arc<super::stripe::auth_stripe::auth_stripe_schema::StripeAccess>>,
    #[serde(skip)]
    pub mercury_access:
        Option<std::sync::Arc<super::mercury::auth_mercury::auth_mercury_schema::MercuryAccess>>,
    pub provider: String,
    pub headers: HashMap<String, String>,
    pub query: HashMap<String, String>,
    pub method: String,
    pub path: String,
    pub raw_query: Option<String>,
    // Issued afresh by authentication hooks; never accepted from HTTP JSON.
    #[serde(skip)]
    pub schema_policy_scope: Option<std::sync::Arc<dog_typedb::isolation::AuthenticatedScope>>,
}

// Type alias for authentication-enabled REST parameters
pub type BusinessParams = AuthParams<RestParams>;
