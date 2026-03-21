use dog_auth::hooks::AuthParams;
use dog_axum::params::RestParams;

pub type BusinessParams = AuthParams<RestParams>;
