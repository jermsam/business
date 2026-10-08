# Business operations

These commands do not enable live billing, replace a database, or charge a customer. Use protected environment files (mode 0600), never command-line secret values. Run from the repository root. Keep admin credentials separate from the runtime service.

## First production setup

1. Take a backup before migrating an existing database. Rehearse the migration on a restored copy first. Pause the scheduler and writes during a cutover; record the deployed commit and schema revision.
2. In an admin environment file, set `TYPEDB_DB=business_prod`. Run `BUSINESS_ENV_FILE=/private/admin.env cargo run --locked --manifest-path server/Cargo.toml --bin database -- create` for a **new** database. This loads the base schema, access policies, private records, billing, onboarding, recovery and history. Existing databases use the explicit `migrate-records`, `migrate-access`, `migrate-billing` commands; schema changes are additive, not destructive rollback scripts.
3. Create the non-admin `business_prod_app` user with `database create-user`, supplying `BUSINESS_NEW_USER` and `BUSINESS_NEW_PASSWORD` through the protected environment. TypeDB's standard user is not database-scoped: protect its credential as cluster-wide data access. Keep admin credentials out of Render.
4. Set `BUSINESS_OWNER_EMAIL` and `BUSINESS_WORKSPACE_DOMAIN` to the real operator's email and lowercase workspace domain. Run `operations bootstrap` against the empty database. It refuses any existing user/company, creates the first owner with an unknown random password, and queues recovery. A fixed unique database key also prevents concurrent first-owner creation. Save the returned merchant UUID as `BILLING_MERCHANT_ID`; it is not a credential. If queuing fails after creation, use `operations recovery-request` rather than trying to create another owner.
5. Configure runtime environment secrets, verified sender, `BILLING_PUBLIC_URL=https://jitpomi.com/account/`, mail and scheduler. Run `operations recovery-tick` to deliver the owner's link; the owner chooses their own password. Sandbox mail is restricted to `BILLING_TEST_EMAIL`. Live recipients require the deliberately selected live configuration. No generated password or recovery token is printed by these commands.
6. With the **runtime** environment, run `operations preflight`. It validates application startup/security configuration, database access, recovery schema, and an active merchant owner. This is a configuration check, not proof of a working public deployment.
7. Deploy the backend first, then the portal. Versioned sessions invalidate pre-upgrade logins once; customers sign in again. Validate public HTTPS, login/logout, reset, cross-tenant denial, webhook signature rejection, scheduled processing, and provider history. Use sandbox fixtures before enabling real customers.
8. Live cutover requires the actual merchant's live Stripe/Mercury credentials, the live Stripe webhook secret, `business_prod`, distinct strong origin/tick/JWT secrets, verified email and explicit `BILLING_LIVE=true`. Do not copy synthetic customers or sandbox provider IDs into production. Existing live-mode startup checks reject test keys and `business_dev`. No live keys or live charges were enabled as part of this implementation.

All operator commands use:

```sh
BUSINESS_ENV_FILE=/private/runtime.env cargo run --locked --manifest-path server/Cargo.toml --bin operations -- preflight
```

## Backups and restoration

Set `BUSINESS_BACKUP_DIR` in the environment to a **new** directory under a trusted private parent. Run `operations backup`. It creates a mode-0700 directory, exports schema and data, restricts files to mode 0600, and writes a SHA-256 manifest only after export completes. A directory without a manifest is incomplete. The exports contain password hashes, invitation/recovery state and customer data; the directory permissions are not encryption. Store retained copies on encrypted storage with restricted access and an agreed retention period. No paid storage is required or provisioned by this tool.

Run before every migration, and schedule backups at the business's chosen recovery-point interval using an existing trusted machine. This implementation supplies the command; it does not promise a provider-managed backup schedule.

To restore, set `TYPEDB_DB` to a new `business_restore_<UUID>` or an absent `business_prod`, retain the backup directory setting, and run `operations restore-new`. Checksums are verified before creating the target. Existing targets are always rejected. An import failure leaves the new target for inspection; the tool never deletes it. Checksums detect corruption, not malicious replacement, so protect the manifest with the backup.

Before changing the application's database, run the policy/ownership comparison rehearsal (`hosted_backup_restore_drill`) and the new-database bootstrap/backup test (`hosted_bootstrap_backup_restore_operations`). Keep the scheduler paused until restored invoice IDs and provider state are reconciled: a restore can resurrect an old pending operation even though the provider already completed it. Do not blindly resume automatic collection after a rollback.

## Recovery and uncertain outcomes

`operations attention` lists opaque IDs and states for recovery emails, unfinished billing notices and uncertain invoice creation. Investigate these daily and after restarts/provider outages. It does not print reset links, email bodies, passwords or provider credentials.

Recovery requests return the same answer for registered and unknown emails. The edge rate-limits requests; TypeDB coalesces each email into ten-minute windows. Links expire one hour after the request and are single-use. Password resets update a credential version and tenant revisions atomically, invalidating old sessions and all earlier links. Confirmation email follows a successful reset. Recovery is serviced by the existing five-minute tick, at most one attempted email per tick; queued delivery may take longer during a backlog or cold start. Expired requests are not sent. A new request is safe after the cooldown.

Email delivery uses durable claims and provider idempotency keys. A crash or uncertain response can leave `sending`, `confirming` or `attention`. Inspect Resend delivery first; do not blindly reset a claim after the provider's idempotency retention window. If a recovery email cannot be established as sent, request a fresh expiring link. Financial notices require an operator review to avoid duplicate receipts/reminders. The tool does not silently retry uncertain sends.

For a disputed or refunded payment, the customer's **View payment details** reads current Stripe invoice-payment, charge, refund and dispute records and saves a snapshot. Invoice balance and payment adjustments remain separate because one payment may fund multiple invoices. Pending/failed refunds retain their provider status. This is read-only history: issue refunds and respond to disputes in the provider dashboard with the merchant's authorized operator account. Mercury currently returns an explicit unavailable message for detailed refund/dispute history; it must not be interpreted as zero refunds/disputes.

## Validation commands

```sh
cargo fmt --manifest-path server/Cargo.toml --check
cargo test --locked --manifest-path server/Cargo.toml --lib
cargo clippy --locked --manifest-path server/Cargo.toml --all-targets -- -D warnings
# Protected environment variables must already be selected for these ignored tests:
cargo test --locked --manifest-path server/Cargo.toml --lib hosted_password_recovery -- --ignored --nocapture
cargo test --locked --manifest-path server/Cargo.toml --lib stripe_sandbox_partial_refund -- --ignored --nocapture
cargo test --locked --manifest-path server/Cargo.toml --lib hosted_invoice_recovery -- --ignored --nocapture
cargo test --locked --manifest-path server/Cargo.toml --lib hosted_bootstrap_backup -- --ignored --nocapture
```

Reference behavior: [OWASP password recovery](https://cheatsheetseries.owasp.org/cheatsheets/Forgot_Password_Cheat_Sheet.html), [Stripe invoice payments](https://docs.stripe.com/api/invoice-payment/list), [attached payments](https://docs.stripe.com/api/invoices/attach_payment), [refunds](https://docs.stripe.com/api/refunds/object), [disputes](https://docs.stripe.com/api/disputes/list), [sandbox fixtures](https://docs.stripe.com/testing).
