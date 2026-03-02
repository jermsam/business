use dog_auth::{AuthenticationService, AuthenticationStrategy};
use dog_auth_local::{LocalEntityResolver, LocalStrategy};
use dog_core::HookContext;
use serde_json::Value;
use std::sync::Arc;

struct TypeDbUserResolver;

#[async_trait::async_trait]
impl<P> LocalEntityResolver<P> for TypeDbUserResolver
where
    P: Send + Clone + 'static,
{
    async fn resolve_entity(
        &self,
        _username: &str,
        _ctx: &mut HookContext<Value, P>,
    ) -> anyhow::Result<Option<Value>> {
       
        // // Example: call a custom method on your users service.
        // let users = ctx.services.service::<Value, P>("accounts")?;
        // // TypeDB query: find one user by email and fetch all its attributes.
        // // This matches the working example:
        // // match
        // //   $u isa user, has email "...";
        // // fetch {
        // //   "u": { $u.* }
        // // };
        // let query = format!(
        //     "match \
        //       $u isa user, has email \"{}\"; \
        //     fetch {{ \"u\": {{ $u.* }} }};",
        //     username
        // );
        // let params = serde_json::json!({
        //     "query": query,
        // });

        // let out = users
        //     .custom(&ctx.tenant, "read", Some(params), ctx.params.clone())
        //     .await?;

        // println!("!!! Raw TypeDB response: {:?}", out);

        // // The TypeDB adapter returns conceptDocuments for this fetch, with a shape like:
        // // { ok: { answers: [ { "u": { "id": "...", "email": "...",
        // //                               "password-hash": "..." } } ], ... } }
        // let mut id_opt: Option<String> = None;
        // let mut email_opt: Option<String> = None;
        // let mut password_hash_opt: Option<String> = None;

        // if let Value::Object(root) = &out {
        //     if let Some(ok) = root.get("ok") {
        //         if let Some(answers) = ok.get("answers").and_then(|v| v.as_array()) {
        //             if let Some(first) = answers.first() {
        //                 // For conceptDocuments, the structure is: answers[0].data.u
        //                 if let Some(data) = first.get("data") {
        //                     if let Some(u) = data.get("u") {
        //                         if let Some(id) = u.get("id").and_then(|v| v.as_str()) {
        //                             id_opt = Some(id.to_string());
        //                         }
        //                         if let Some(e) = u.get("email").and_then(|v| v.as_str()) {
        //                             email_opt = Some(e.to_string());
        //                         }
        //                         if let Some(ph) = u.get("password-hash").and_then(|v| v.as_str()) {
        //                             password_hash_opt = Some(ph.to_string());
        //                         }
        //                     }
        //                 }
        //             }
        //         }
        //     }
        // }

        // let id = match id_opt {
        //     Some(i) => i,
        //     None => return Ok(None),
        // };

        // let email = match email_opt {
        //     Some(e) => e,
        //     None => return Ok(None),
        // };

        // let password_hash = match password_hash_opt {
        //     Some(h) => h,
        //     None => return Ok(None),
        // };

        // let user = serde_json::json!({
        //     "id": id,
        //     "email": email,
        //     // Map TypeDB's password-hash attribute into the logical password field
        //     // expected by LocalStrategy (which defaults entity_password_field to "password").
        //     "password": password_hash,
        // });

        // println!("[TypeDbUserResolver] resolved user: {:?}", user);

        // Ok(Some(user))
        
        Ok(Some(serde_json::json!({})))
    }
}

pub(crate) fn register_local<P: Send + Clone + 'static>(
    auth: Arc<AuthenticationService<P>>,
) -> Arc<LocalStrategy<P>> {
    let strategy = Arc::new(
        LocalStrategy::new(&auth.base)
            .with_entity_resolver(Arc::new(TypeDbUserResolver)),
    );
    let strategy_trait: Arc<dyn AuthenticationStrategy<P>> =
        Arc::clone(&strategy) as Arc<dyn AuthenticationStrategy<P>>;
    auth.register_strategy("local", strategy_trait);
    strategy
}