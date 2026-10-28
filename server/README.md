# Business backend

One DogRS application, one canonical TypeDB model, one executable: `server`.
There is no alternate rebuild server or legacy fallback.

Human first. Simple first. Local first.

## Structure and responsibilities

- `src/app.rs` composes the same application used by the executable and integration tests.
- `src/auth/` uses dog-auth/dog-auth-local for credentials, JWT verification and persisted revocation. Current identity, credential version and tenant membership are checked for each protected call.
- `src/services/data/` exposes only custom `read` and `write`. The frontend sends complete TypeQL. Hooks validate the transport envelope and bind freshly verified scope; dog-typedb's schema-policy executor enforces the model. There is no per-query registry.
- `src/schema.tql`, `src/functions.tql`, `src/schema-policy.tql` and `src/billing.tql` define identities, membership, ownership, grants, reviews and domain rules. Security functions and annotations are not replaced with Rust domain switches.
- Provider resources have `_service`, `_shared`, `_hooks` and `_schema` modules; common provider hooks live in `hooks/`, credentials in `auth-<provider>/`, and HTTP in adapters. Providers do not query TypeDB.
- `src/database.rs` and the `database` CLI handle explicit provisioning. Startup verifies the canonical model and never migrates, seeds or wipes data.
- Tests live under `tests/`; no dormant test files for removed services are presented as coverage.

The backend exposes authentication, protected data queries and provider proxies.
An opt-in server-owned billing worker now executes the canonical billing graph;
see **Billing execution** below. Provider proxies remain transport-only. Removed
onboarding/recovery/payment-demo routes, store checkout coordination and Gelato
fulfillment have not been restored.

## Provisioning and launch

Use a private environment file outside Git (permissions 0600). See `.env.example`.
Runtime requires non-admin TypeDB credentials, TLS, an existing canonical database
and `AUTH_JWT_SECRET` of at least 32 nonblank bytes.

Use separate operator credentials to provision a **new** target such as
`business_native_dev` when an existing `business_dev` still contains the old model:

```sh
BUSINESS_ENV_FILE=/private/admin.env cargo run --locked --bin database -- create
BUSINESS_ENV_FILE=/private/admin.env cargo run --locked --bin database -- verify
BUSINESS_ENV_FILE=/private/admin.env cargo run --locked --bin database -- bootstrap
BUSINESS_ENV_FILE=/private/runtime.env cargo run --locked --bin server
```

Both environment files must explicitly select the intended `TYPEDB_DB`.
Bootstrap additionally requires `BUSINESS_OWNER_EMAIL`, `BUSINESS_OWNER_NAME`,
`BUSINESS_TENANT_NAME` and `BUSINESS_OWNER_PASSWORD` (12–72 bytes). It works
only on an empty identity registry and uses a unique bootstrap record to arbitrate
concurrent attempts. It prints tenant/identity IDs, never credentials.
Create refuses an existing target; unknown commands fail before connecting.
`list` is read-only; `create-user` requires operator-supplied
`BUSINESS_NEW_USER` and `BUSINESS_NEW_PASSWORD` and never replaces an account.

There is no destructive reset or automatic migration from the old model. Preserve
existing databases and reconcile any provider records before an independently
approved cutover. The Render blueprint targets `business_native_dev`, disables
automatic deployment and keeps credentials external. Changing that file does not
provision or deploy anything.
Any previously deployed billing scheduler must be retired as an explicit
deployment action; removing its source does not disable a remote cron job.

## HTTP contract

Use `Authorization: Bearer <token>` and `x-tenant-id` on protected calls.
Local sign-in is POST `/authentication` with strategy `local`, email and password.
For an email-only portal, strategy `account` privately discovers an active tenant
membership and verifies the same password before returning its tenant ID. No
company or identity information is returned for failed sign-ins. Explicit tenant
sign-in and fresh membership checks on protected calls remain unchanged.
DELETE `/authentication` revokes the supplied token durably.

POST `/data` with `x-service-method: read` or `write` and only
`{"query":"complete TypeQL"}`. The response is the DogRS
`ok.answers` envelope, never a raw query echo. Credentials/revocation fields are
hidden by the model. The publicly served `/queries/data/*.tql` files are examples
for the browser, not an authorization registry; `/data-client.js` sends queries.

`/health` is public. API execution has a per-process active budget
(`BUSINESS_ACTIVE_REQUESTS`, 1–32), 32 waiting slots and a five-second admission
deadline. Overload returns 503 without starting the operation.
If an additional trusted gateway is used, configure `BUSINESS_GATEWAY_SECRET`
(at least 32 bytes) and supply `x-gateway-secret` on API requests. This is
additional protection, never a replacement for tenant authentication.

## Verification

```sh
cargo build --locked --all-targets
cargo test --locked --all-targets
cargo test --locked --doc
cargo clippy --locked --all-targets -- -D warnings
node --test tests/frontend/data-client.test.mjs
BUSINESS_ENV_FILE=/private/dev.env BUSINESS_RUNTIME_ENV_FILE=/private/runtime.env cargo test --locked --test backend -- --ignored --test-threads=1
```

The real integration suite uses existing development-cluster credentials and
generated `business_backend_test_<UUID>` databases only. It checks RBAC
decisions, frontend queries, hidden data, rollback, concurrency, credential rotation
and membership expiry. The executable test provisions and bootstraps a disposable
target using the actual CLI, starts the actual server with non-admin credentials,
then checks gateway enforcement, owner login, data writes/reads and logout.
Disposable targets are cleaned up; configured databases are never deleted.
Provider sandbox tests are separately gated and never imply live-money acceptance.
Mercury sandbox access currently has an external IP-allowlist restriction.

### Verification evidence — October 20, 2026

The 0.3.4 follow-up reproduces absent-policy deletion as HTTP 500 on the old
dependency and verifies HTTP 403, client redaction and unchanged persisted state
on the published fix. All four real backend tests, default tests, Clippy and
frontend tests pass with the published dependency and no local override. Real
Stripe test-mode HTTP acceptance covers authenticated customer CRUD/idempotency,
unconfirmed intent cancellation, Checkout expiration and draft invoice/item
reads with synthetic cleanup. No payment is confirmed or email sent. Mercury's
direct sandbox baseline still returns HTTP 401 with an IP restriction; do not
describe that as passing provider acceptance. See `tests/README.md` to rerun the
explicit provider checks.

All-target debug builds, release builds of both executables, formatting and
Clippy with warnings denied pass. The final published-0.3.3 dependency run passed
29 library tests, two layout tests, all four real TypeDB backend tests,
and three frontend-client tests. The real Stripe test-mode lifecycle passed in
the earlier consolidation run; it was not repeated for the compiler-only patch.
All Rust source/test files are reachable from Cargo targets; disconnected
modules and old test files are no longer silently outside build coverage.
Generated test databases were cleaned up. No production database, live payment,
deployment or external credential configuration was changed.

### Review failure investigation and fix

The first review 500 lost its underlying cause, so later passing runs were not
accepted as a resolution. Focused concurrent reviews subsequently reproduced a
30-second pre-commit deadline failure without fault injection. A controlled
probe then added 250 ms to each compiler schema-introspection round trip: the
old compiler made 42 such requests before its first mutation, and all four
concurrent reviews returned the published executor's 500 at about 31 seconds.
Compilation, not execution of the mutation, consumed most of the budget.

dog-typedb 0.3.3 fixes that excessive introspection by reusing immutable schema
metadata inside the same transaction and across its checkpoints. It never caches
frontend queries, policy decisions, authentication, membership or application
data, and never reuses metadata across transactions. The identical latency probe
needed 9 lookups for the first checkpoint and 12 for the entire review write;
attempts completed in roughly 6–8 seconds without increasing the 30-second limit.
The probe deliberately kept the old timeout-to-500 mapping, demonstrating that
the result is not merely a changed HTTP error classification. No write was retried.

The fix also retains private error causes and execution phases, keeps them out
of client responses, and returns 409 only for TypeDB's structured STC2 commit
rejection. Lost connections and commit deadlines remain unknown outcomes; the
framework never automatically replays them. The tenant revision update remains
in place, preserving conflict detection between concurrent policy changes.

Regression coverage includes real compiler round-trip reuse and fresh transaction
metadata, nine distinct reviews, successful-repeat deduplication, denied self-review
with unchanged history/revision, concurrent authenticated service calls, real HTTP
reviews, and reconciliation of every response against persisted history. Before
publication, all five framework live tests and all four Business backend acceptance
tests passed (Business: 187.46 seconds). The original historical request cannot be
retroactively attributed with certainty because its cause was discarded; the
reproduced review timeout now has a demonstrated code fix and before/after evidence.

dog-typedb 0.3.3 is published on crates.io and pinned exactly in Business's
manifest and registry lockfile. A fresh run against the downloaded release,
without a local patch, passed all four backend tests in 186.74 seconds. Debug
and release builds, unit/layout/frontend tests, formatting and all-target Clippy
also pass. No dependency other than dog-typedb changed during this patch upgrade.
No application deployment or live payment was performed.

The real Mercury sandbox check remains blocked: both the service chain and a
direct accounts request were rejected by the provider's IP allowlist (direct
HTTP 401). Local Mercury auth, resource and HTTPS contracts pass. Do not call
this external acceptance check green or weaken the allowlist to bypass it.

## Model references

The model follows [TypeDB's RBAC in Business guide](https://typedb.com/docs/learn-typedb/rbac-in-business/).
Service composition follows the
[DogRS examples](https://github.com/Jitpomi/dogrs/tree/main/dog-examples),
especially fleet-queue and social-typedb. TypeDB functions alone are not an
automatic query authorization boundary: the protected data service always uses
dog-typedb's authenticated schema-policy executor.

## Stripe provider services

Stripe follows the same provider layout as Mercury. Each resource has its own
service, shared hook registration, hooks and schema. Shared authentication and
request-safety hooks are in `src/services/stripe/hooks/`. Credentials live only
in `auth-stripe`; the adapter contains HTTP/endpoint/form/error handling, not
TypeDB queries, invoice workflows or application business rules.

| Service under /stripe/ | Methods beyond find/get |
| --- | --- |
| customers | create, patch, remove, payment_methods |
| invoices | create, patch, remove (draft), finalize, pay, send, void, mark_uncollectible, attach_payment |
| invoice-items | create, patch, remove |
| checkout-sessions | create, patch, expire, line_items |
| payment-methods | create (tokenized card), patch, attach, detach |
| setup-intents | create, patch, confirm, cancel |
| payment-intents | create, patch, confirm, cancel, capture |
| invoice-payments | read-only |
| charges | patch; use PaymentIntents instead of deprecated charge creation |
| refunds | create, patch, cancel |
| disputes | patch, close |
| mandates | get only |

Configure `STRIPE_CONNECTIONS_JSON` as an external secret containing an array of
connections, each with `tenant_id`, `identity_ids`, `environment` (`test` or
`live`), `token`, optional `connected_account`, and `write` (defaults false).
No fallback global key is granted to every tenant. Configure each tenant against
its own Stripe account/Connect account: JWT tenant checks do not partition
customers within a shared upstream Stripe account. API keys are server-only.
A configured Connect account cannot be overridden with client headers.
The existing API version `2026-03-25.dahlia` is preserved.

All resource calls reauthenticate using dog-auth, resolve credentials through the
registered auth-stripe hooks, and run resource validation. Auth-stripe has no
public route. Use a Bearer token and `x-tenant-id` for HTTP calls. Custom methods
use POST to the collection with `x-service-method` and an `id` in the JSON body.
Stripe partial updates use PATCH, not PUT. Unsupported HTTP methods return 405.

Every provider POST requires a stable `Idempotency-Key`. It is namespaced by
tenant, environment, Connect account and endpoint; retries of the same operation
must keep the original key and body. Stripe's retention window is not permanent
exactly-once delivery. GET and DELETE do not send the key. There are no automatic
write retries or redirects. Reconcile uncertain outcomes before retrying. Amounts
are integer minor units, never floating-point money. Raw card/bank credentials
are rejected; collect them with Stripe-hosted UI or Stripe.js.

Find returns one page, not a settled total. Use `limit` (1–100) and the last
returned ID as `starting_after`; continue until an empty page. Custom list
methods preserve Stripe's list envelope and `has_more`. These are provider
proxies, not a completed storefront, webhook receiver or fulfillment workflow.

Tests live in `tests/stripe/`:
`cargo test --locked --lib stripe` exercises registered JWT/auth/resource hooks,
resource/action endpoint mapping, tenant separation, validation, idempotency and
real local HTTPS transport. The ignored `real_stripe_test_customer_lifecycle`
requires `BUSINESS_ENV_FILE` pointing to an external dotenv file with
`STRIPE_SECRET_KEY` in TEST mode. It creates synthetic resources only, never
confirms a payment, sends an invoice, or uses real money. Real sandbox coverage
does not imply live-account or all-action acceptance.


## Mercury provider services

Mercury follows the resource-service/shared-adapter pattern. Resource services
delegate to `src/services/mercury/adapters/mercury_adapter.rs`; that adapter makes
Mercury HTTP calls only. It contains no TypeDB queries or billing workflows.

| Service | Methods |
| --- | --- |
| `mercury/accounts` | `find`, `get` |
| `mercury/customers` | `find`, `get`, `create`, `update`, `remove` |
| `mercury/invoices` | `find`, `get`, `create`, `update`, custom `cancel`, custom `pdf` |
| `mercury/transactions` | `find`, `get` |

Mercury's POST edit endpoints require complete resource data. They are **update**,
not patch. Invoice updates must explicitly retain or clear optional fields.
Neither accounts nor transactions expose mutation methods. Customer deletion
requires one UUID; bulk deletion is forbidden. Invoices are cancelled, not deleted.
Account `get` searches the documented paginated accounts API: it does not invent
an undocumented single-account endpoint. `find` returns one page of records;
use `limit` (1–1000), `order`, `start_after` or `end_before` to page explicitly.
Transaction filters are documented in `transactions/transactions_schema.rs`; array filters use
comma-separated values in REST query parameters and become repeated upstream keys.

Each resource registers strict dog-auth JWT authentication, the
`auth-mercury` credential-resolution hook, and its own request/response validators.
The resource's `_schema.rs` defines its contract and its `_hooks.rs` invokes those
checks. There is no shared Mercury resource-validation dispatcher.
Mercury-wide resource security hooks live in `mercury/hooks/`: one registration
helper installs strict Business authentication followed by credential resolution
through the registered `auth-mercury` service. Resource validation follows these
shared hooks; credential storage and auth-service-specific hooks stay in
`auth-mercury/`.
Internal calls require a fresh bearer token too. No request can choose an API host,
provider token or model. Credentials are server-only and excluded from serialized
parameters and debug output. No global Mercury token is shared across arbitrary
tenants. Connection access is explicit and read-only unless configured otherwise.

`auth-mercury/` has its own service, hooks, shared definitions and schema. Its
server-only custom `resolve` method resolves the connection for the freshly
verified Business tenant/identity. Resource hooks call it through
`ctx.services.app().service(...)`, so the auth service's hooks run; they do not
access the raw service registry. It has no public HTTP route and explicitly
rejects external-provider calls. Tokens are returned only to the internal caller
and bound into non-serializable, debug-redacted request credentials. The Mercury
HTTP adapter owns neither the credential store nor resource request validation.
The existing connection mode is API-token authentication: these tokens are
configured by the operator, not obtained/refreshed through an invented OAuth flow.

Set `MERCURY_CONNECTIONS_JSON` in the external secret environment file to a JSON
array with `tenant_id`, `identity_ids` (array), `environment` (`sandbox` or
`production`), `token`, and `write` (default false). Use the exact verified identity
IDs and a token created in the corresponding Mercury environment. One connection
per tenant is currently supported. Omit this setting to leave Mercury unregistered.
This is deployment credential configuration, not a TypeQL query registry.

Standard methods use the corresponding REST verbs (`PUT` for full `update`).
Custom methods use `POST /mercury/invoices`, `x-service-method: cancel` or `pdf`,
and body `{ "id": "<invoice UUID>" }`, together with the normal bearer token and
`x-tenant-id` headers. GET cannot trigger cancellation. PDF results are a lossless
JSON envelope with `contentType`, `encoding: "base64"`, and `data`.

Creating an invoice requires an explicit `sendEmailOption` (`DontSend` or
`SendNow`), because Mercury otherwise sends it immediately. Requests do not round
money, silently merge partial updates, follow redirects, or retry mutations.
After an uncertain result, reconcile against Mercury before retrying. Do not
assume Mercury implements an idempotency guarantee that its endpoint does not
document. JSON and PDF response sizes are bounded; provider error bodies and
credentials are not exposed to callers.

Run `cargo test --locked --lib mercury` for the provider module tests, or
`cargo test --locked --lib` for all currently registered unit tests. Tests are in
`tests/mercury/` and exercise registered DogRS services, real JWT verification,
HTTP routing, and actual local HTTPS traffic. They never call Mercury or send
real invoices. A real sandbox smoke check needs a separately configured sandbox
token; local fixture tests are not evidence of live account/API access.

Check every backend target with `cargo check --locked --all-targets`, build with
`cargo build --locked --all-targets`, and run the registered backend suite with
`cargo test --locked --all-targets`. The obsolete operations executable, module,
tests and runbook have been removed; no database or provider data was removed.
Deleted billing/onboarding/payment services have not been restored to make the
provider services compile. External sandbox and disposable TypeDB tests are
explicitly ignored by default and must be run separately with development
credentials; a green local suite is not proof of production acceptance.

References: [Mercury invoicing](https://docs.mercury.com/docs/invoicing),
[full invoice updates](https://docs.mercury.com/reference/updateinvoice),
[sandbox setup](https://docs.mercury.com/docs/using-mercury-sandbox).

This application uses published dog-typedb 0.3.3, dog-auth 0.3.1, and other DogRS 0.3.0 crates, dog-transport HTTP hosted by
Axum, and TypeDB 3.13.6 (the driver version required by dog-typedb).

Development and production use the same code, TLS validation, auth strategy and
durable TypeDB token store. Set `BUSINESS_ENV_FILE` to an owner-readable file
outside the project. The prepared configurations are in
`~/.config/jitpomi/business/dev.env` and `prod.env`. Their credentials must not be
committed or pasted into logs. Use a dedicated non-admin runtime account.

## Billing execution

`src/billing/` schedules invoices, reconciles provider status, attempts authorized
Stripe payments, verifies Stripe webhooks, and delivers invoice/receipt/reminder
emails through Resend. It uses the current `tenant`/`bill_account` graph, not the
removed company/workspace model. Frontend reads and permitted plan changes still
use protected `/data`; private provider identifiers and consent remain hidden.
This change does not restore the old portal's onboarding or saved-card endpoints.

Enable explicitly with `BILLING_ENABLED=true`, `BILLING_MERCHANT_ID` (tenant UUID)
and `BILLING_TICK_SECRET` (32+ characters). The executable runs a bounded pass every
60 seconds. One worker instance handles one explicitly configured merchant; other
tenants are not swept. Use a separate worker configuration for another merchant.
Provider credentials are selected from that tenant's existing
`STRIPE_CONNECTIONS_JSON` / `MERCURY_CONNECTIONS_JSON`; write permission and matching
environment are required. A configured Stripe Connect account is applied to all
Stripe requests and checked on incoming events. Global legacy provider keys are
not used by the engine.

- `POST /internal/billing/tick`: bearer tick secret; no tenant header can change
  its merchant. A pass is bounded to ten new occurrences, ten issuance attempts,
  twenty reconciliations and twenty notification candidates.
- `POST /webhooks/stripe`: raw-body signature verification with a five-minute
  timestamp tolerance and the configured `STRIPE_WEBHOOK_SECRET`. The event only
  prompts a fresh provider read. Stored invoice ID, merchant ownership, customer,
  amount, currency, metadata and environment must agree. Repeated events cannot
  undo a paid/void state or initiate a charge. Receipts are picked up by the worker.
- `BILLING_MAIL_ENABLED=true` enables email with `RESEND_API_KEY` and
  `BILLING_EMAIL_FROM`. Sandbox delivery requires an exact `BILLING_TEST_EMAIL`
  match and is marked TEST. Mercury invoice creation uses `DontSend`, so this same
  delivery policy also governs its invoice emails. `BILLING_MERCURY_DESTINATION`
  selects that merchant's deposit account.

Amounts are integer USD cents. Weekly/monthly/yearly schedules use UTC and retain
the original anchor (January 31 → February 28 → March 31). One-time plans finish
once. Schedule advancement and invoice insertion commit together. Tenant revision
writes arbitrate overlapping workers; every mutation requires exactly one result
before commit. A changed/missing consent revision, manual customer choice, manual
merchant policy or paused plan prevents automatic collection. Admin preference
alone never grants consent.

Stripe auto-advance is off. The worker makes at most one off-session attempt per
invoice, after a durable claim, using a stable idempotency key. Declines leave a
payment link for the customer. Processing bank payments receive no pay-again
reminder. Open due invoices can receive at most four reminder windows, separated
by `BILLING_REMINDER_DAYS` (1–30, default 7). A paid invoice gets one receipt.
Resend acceptance is recorded with its message ID; acceptance is not itself proof
of inbox delivery.

### Uncertain outcomes and operations

Do not reset durable claims to retry blindly. An `issuing`/`attention` invoice with
no external ID needs provider-side lookup by `business_invoice` metadata (Stripe)
or `JIT-<invoice UUID>` (Mercury). A known draft needs operator reconciliation
before finalization; the worker does not recreate it. A Mercury customer marked
`pending` similarly needs reconciliation because customer creation has no assumed
idempotency contract. A notice in `sending` or `attention` must be checked with
Resend before any re-send. These states deliberately stop ambiguous external
side effects. They are not automatically repaired by restarting the worker.

Keep the old remote scheduler disabled before any cutover, reconcile outstanding
provider records, and configure the new worker against a verified canonical
database. Startup does not migrate data, configure a remote webhook, or enable
live payments. Mercury's restored sandbox connection still needs its hosted card
checkout checked separately; Stripe test acceptance does not certify Mercury.

### Sandbox workflow acceptance

Run the explicitly ignored `scheduled_payment_receipt_and_decline_reminder` test
in `tests/billing/main.rs` with `BUSINESS_ENV_FILE` (development operator source,
`TYPEDB_DB=business_dev`) and `BILLING_TEST_ENV` (protected Stripe test key and
approved Resend test configuration). It creates/deletes only a new
`business_backend_test_<UUID>` database. Optional `BILLING_TEST_REPORT` writes
non-secret evidence. Plaintext TypeDB is permitted only for loopback test
instances; normal runtime continues requiring TLS. The provider fixtures are
synthetic and remain in Stripe sandbox for inspection.

```sh
cargo test --locked --test billing -- --ignored --nocapture --test-threads=1
```

This test actually pays a $1 sandbox invoice, declines another, verifies the
receipt/reminder records, and checks repeated ticks, concurrent scheduling,
revoked consent, paused plans, provider binding, tenant isolation and signed
webhook replay. It sends clearly marked test emails only to dev@jitpomi.com.

### Sandbox deployment recovery — October 27, 2026

The free TypeDB Cloud validation cluster had automatically paused. Resuming the
existing cluster restored connectivity without changing its address, credentials
or stored data. `business_native_dev` passes canonical schema verification.
The deployed billing merchant must match the tenant in that database and the
provider connection configuration; legacy company IDs are not interchangeable.

Read-only provider connections remain available to authenticated provider reads,
but are not selected as billing executors. A read-only Mercury connection does
not prevent a writable Stripe test connection from starting the worker. Mode
mismatches on writable connections still fail startup. No permission is expanded.

The legacy website portal still requires adaptation to the canonical data-query
contract and an explicit account migration or fresh sandbox provisioning. Backend
health and provider workflow tests alone do not establish website acceptance.
