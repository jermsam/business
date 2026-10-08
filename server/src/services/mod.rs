use dog_core::DogAppBuilder;
use serde_json::Value;
use std::sync::Arc;
mod types;
pub use types::*;
mod apps;
mod authentication;
mod memberships;
mod policy_state;
mod records;
mod workspace;

pub fn configure(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<authentication::AuthService>,
    state: Arc<crate::typedb::TypeDBState>,
) -> anyhow::Result<()> {
    builder.register_service("authentication", auth.clone());
    builder.register_service(
        "records",
        Arc::new(records::RecordsService::new(state.clone(), auth.clone())),
    );
    builder.register_service(
        "workspace-records",
        Arc::new(workspace::WorkspaceService::new(
            state.clone(),
            auth.clone(),
        )),
    );
    builder.register_service(
        "memberships",
        Arc::new(memberships::MembershipsService(
            workspace::WorkspaceService::new(state.clone(), auth.clone()),
        )),
    );
    for (name, target) in [
        ("access-grants", policy_state::Target::Grant),
        ("entitlements", policy_state::Target::Entitlement),
        ("team-memberships", policy_state::Target::TeamMembership),
    ] {
        builder.register_service(
            name,
            Arc::new(policy_state::PolicyStateService(
                workspace::WorkspaceService::new(state.clone(), auth.clone()),
                target,
            )),
        );
    }
    builder.register_service(
        "apps",
        Arc::new(apps::AppsService(workspace::WorkspaceService::new(
            state, auth,
        ))),
    );
    authentication::authentication_shared::register_hooks(builder)?;
    Ok(())
}
