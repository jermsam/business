# Shared payments: incremental extraction

Status: sandbox endpoints and owner-only browser panel prepared for hosted testing. Not approved for live checkout. SEYFR is disconnected.

## Hosted browser testing (2026-10-16)

Browser entry point: `https://jitpomi.com/account/payments/`. Sign in through
`/account/` and select the existing merchant company first. The panel uses the
portal's encrypted HttpOnly session; no Stripe key, application bearer token, or
event signing secret is sent to the browser. Each action rechecks the account's
active owner membership in TypeDB. Other companies and buyers are rejected.

The panel creates USD 0.01–100 sandbox-only checkouts, verifies Stripe payment
state, displays status, and explicitly dispatches the paid event. It retains only
test operation/session identifiers in tab-scoped sessionStorage so a return from
Stripe can resume the same operation. Do not clear storage or start another test
to resolve an uncertain charge; inspect status first.

The configured HTTPS demonstration receiver verifies the raw-body HMAC, timestamp,
merchant/product/environment, and the locally recorded paid operation and event.
It durably deduplicates `pay_sandbox_receipt` records before acknowledging. It does
not create orders, invoices, emails, shipments, or Gelato jobs. This is a demo
consumer, not evidence of an independently deployed consumer's acceptance.

Deployment requires migration 009 on **business_dev only**, an explicit application
grant, and protected `PAYMENTS_SANDBOX_ENABLED`, `PAYMENTS_BROWSER_APP`, and
`PAYMENTS_APPLICATIONS_JSON` configuration. `prepare-payment-sandbox` generates an
owner-only config file using the existing approved sandbox merchant; the explicit
`provision-payment-sandbox` database command inserts its grant without changing
other accounts. Never commit the generated config or enable live billing here.

## Current checkpoint: authenticated endpoints and acceptance (2026-10-16)

Added sandbox-only service endpoints:

| POST route | Responsibility |
| --- | --- |
| `/payments/v1/checkouts` | Create/replay one-time hosted checkout |
| `/payments/v1/status` | Authorized operation status |
| `/payments/v1/reconcile` | Fetch Stripe evidence and atomically record paid/event |
| `/payments/v1/events/dispatch` | Deliver that paid operation's event to its configured receiver |

The opt-in `PAYMENTS_SANDBOX_ENABLED=true` enables routing in the server binary.
It is false/unset by default; the router refuses `business_prod`. Each entry in
protected `PAYMENTS_APPLICATIONS_JSON` config supplies application ID, credential
SHA-256 digest, merchant, product, approved HTTPS success/cancel URLs and optional
`delivery` HTTPS URL/signing secret. No browser should receive service credentials.
The bearer credential selects the application; merchant/product/environment come
from configuration, not tenant headers or JSON. TypeDB rechecks the active grant.
Unknown body fields fail closed, including scope/URL injection. Digests must be
unique; raw credentials must be high-entropy and at least 32 characters. Return
URLs and receiver secrets are not caller-editable. Body size is capped at 8 KiB;
concurrent endpoint work is capped at four per process. This is not a global
distributed rate limiter or a substitute for hosted admission testing.

Input for all routes (the receiver URL is NOT an input):

```json
{"operation_id":"order-uuid","reference":"product-order-id","amount_minor":100,"currency":"USD","currency_exponent":2}
```

Reconcile additionally accepts `session_id`. Dispatch accepts only a previously
paid, matching operation, and constructs its event ID internally. Successful
delivery retries return delivered status without transmitting again. Event retry
backoff/leases still apply to failed deliveries. The caller's durable scheduler
can invoke dispatch; no remote scheduler has been configured by this work.

`cargo run --locked --bin database -- migrate-payments` is now an explicit
development-only migration command (load the protected development env file).
Existing company/access schema must already be present. It is not run at startup
or as part of billing migration. The hosted sandbox preparation applies it to
business_dev, never business_prod. Earlier acceptance used unique
`payments_test_<uuid>` databases, then deleted only those disposable databases.
Application/grant provisioning remains an explicit operator
step; do not expose schema or arbitrary-query access to product clients.

New acceptance evidence:

- Committed TypeDB, eight simultaneous creates: one request and initial event.
- Eight simultaneous operation claims: exactly one successful claim.
- Eight simultaneous paid-state writes: exactly one transition and paid event.
- Eight simultaneous delivery claims: exactly one lease winner. This concurrency
  fixture uses synthetic provider evidence, not eight Stripe charges.
- Anonymous/invalid bearer requests rejected; caller scope injection rejected;
  changed amounts conflict; revocation prevents further reads.
- Actual authenticated Checkout endpoint -> Stripe sandbox test-card payment in
  hosted browser UI -> authenticated reconciliation -> atomic paid event ->
  authenticated dispatch -> local HTTPS receiver with certificate verification.
- The receiver commits a receipt then deliberately returns 503 (lost ACK).
  A second delivery reuses the stable ID/body and produces one durable receipt;
  sender records delivered. Forged event signatures return 401. A test-only CA is
  explicitly trusted for localhost: no insecure TLS option exists in production.
- No live funds, saved payment methods, customer records, emails or Gelato orders.
  Paid sandbox transactions remain in Stripe's sandbox audit history.

```sh
BUSINESS_ENV_FILE=/absolute/protected/dev.env cargo test --locked --lib committed_payment_concurrency_and_authentication -- --ignored --nocapture
BUSINESS_ENV_FILE=/absolute/protected/dev.env PAYMENTS_SANDBOX_ENV_FILE=/absolute/protected/sandbox.env cargo test --locked --lib sandbox_payment_to_https_receiver -- --ignored --nocapture
```

The full flow prints a fresh sandbox checkout URL, waits up to eight minutes for
an official Stripe test card, and never accepts a live key. Each run uses a unique
operation ID so Stripe idempotency cannot accidentally reuse an earlier test's
paid session. Use no real card, contact details or customer data. These HTTP
endpoint tests execute Axum in-process; only Stripe Checkout and the TLS receiver
use network sockets. They do not certify a hosted reverse proxy deployment.

Remaining before production: live-provider configuration/review, hosted ingress
and admission checks, webhook-driven or monitored reconciliation scheduling,
refund/dispute event handling, unknown/expired checkout recovery, historical
reconciliation after application revocation, secret rotation, outbox monitoring
and operator recovery. Sandbox SEYFR integration must be separately requested;
do not infer that production billing is ready from these acceptance tests.

## Ownership

Payments owns provider transport/authenticity, scoped payment operations, verified
financial observations, refunds, reconciliation, and durable integration events.
Invoicing owns invoices, schedules and recurring-payment consent. Booking owns
reservations and expiry. SEYFR owns merchandise, quotes, print files, QR attribution,
and Gelato fulfilment. No product must create a company billing account solely to
collect a one-time consumer payment. The customer portal remains a client, not the
payment authority. Gelato is a fulfilment provider, not a payment processor.

Do not require all products to use one database or expose database access to clients.
Use versioned service contracts. Keep these boundaries inside the existing server
initially rather than introducing unnecessary independently deployed microservices.

## Implemented first slice

- Billing delegates its existing Stripe calls to `services::payments::stripe`.
- Stripe raw-body signature verification lives in `payments::webhook`; the old
  billing import remains compatible.
- `OperationScope` defines stable, distinct merchant/product/environment/operation
  identities for future consumers. It is not yet wired into existing billing keys:
  changing those keys could duplicate already-issued provider operations.
- No new public routes, database migration, customer-data movement, or live calls.

## Historical second local slice

Added `PaymentRequest` with integer minor amounts, explicit currency/exponent and
stable payload fingerprints. Unknown fields (including raw card fields) fail
deserialization. Product references are identifiers, not arbitrary customer data.
Currency syntax is validated here; authoritative currency/exponent and provider
support validation remains a checkout-adapter requirement, not implemented yet.

Migration `009-payment-platform.tql` is an additive DRAFT. It models registered
applications, active merchant/product/environment grants, immutable payment
requests, and initial request events. A TypeQL authorization function scopes every
repository operation. Request and event insertion share the guarded merchant
write transaction. Claiming checks both fingerprint and requested state.
Replay lookup rechecks the active grant and rejects changed payloads.

This migration is deliberately excluded from existing provisioning commands.
Rust unit tests cover validation, fingerprints and generated query structure.
Real development TypeDB execution is now covered by the rollback test below;
it does NOT prove cross-transaction concurrency. Before use, validate concurrent
fixtures, including overlapping grants, revoked grants, changed payloads,
concurrent creates/claims and uncertain commits. Grant provisioning/revocation
must also use the merchant guarded-write protocol. No external caller can invoke
these helpers through a public API yet. The initial event is not a delivered
outbox in the original slice. The subsequent checkout/outbox implementation is
described below. Refund events remain unimplemented. No existing invoices have
been migrated.

## Checkout and event delivery slice (2026-10-16)

Implemented internal `checkout::start` / `reconcile` helpers, not public routes.
Checkout uses Stripe payment mode, never setup mode; only explicitly enumerated
currency/exponent combinations are accepted. That list is an application policy,
not proof of account-level acceptance for every currency. Server-owned return URLs
are required. Durable request/claim precedes provider calls. Existing checkout
bindings replay without creating another session; processing/unknown outcomes
require reconciliation rather than blind recreation. Paid reconciliation fetches
Stripe state and checks session binding, reference, environment, amount and currency
before atomically transitioning the request and creating one `payment.paid` event.

Implemented internal outbox dispatcher with scope-specific bounded polling,
60-second leases, 10-second HTTPS delivery timeout, no redirects/HTTP retries,
HMAC-signed bodies and timestamps, acknowledgements guarded by lease/attempt,
exponential backoff, eight-attempt dead letters and final-attempt crash recovery.
Delivery is at least once, NOT exactly once: consumers must verify signatures,
validate merchant/product/environment and event type, persist/deduplicate event IDs,
then acknowledge. `payment.requested` never authorizes fulfilment. Destination URLs
and secrets must come from trusted server configuration, never customer requests.

Validation evidence:

- `typedb_payment_queries_rollback`: real `business_dev` schema transaction. Loaded
  migration 009; executed create, replay/fingerprint rejection, app/environment
  isolation, claim replay rejection, checkout binding/replay lookup, paid-event
  deduplication, outbox lease/retry/stale acknowledgement/delivery acknowledgement,
  due selection, final-attempt crash dead-lettering, and grant revocation. All
  synthetic fixtures and schema changes are rolled back. This caught and fixed
  TypeQL value-vs-attribute typing errors that Rust-only tests missed.
- `sandbox_one_time_checkout`: actual Stripe sandbox create/retrieve/expire smoke
  test. Verified payment mode, USD 100 minor units, sandbox and unpaid state. No
  charge, no saved card and no real customer. Session expired after inspection.
- These are separate tests, not a complete browser -> payment -> receiver test.

Reproduce explicitly (never production credentials):

```sh
BUSINESS_ENV_FILE=/absolute/protected/dev.env cargo test --locked --lib typedb_payment_queries_rollback -- --ignored --nocapture
PAYMENTS_SANDBOX_ENV_FILE=/absolute/protected/sandbox.env cargo test --locked --lib sandbox_one_time_checkout -- --ignored --nocapture
```

Remaining at that earlier checkpoint before enabling routes/schedulers: authenticated application admission,
provisioning/revocation and return-URL/destination configuration; committed isolated
development concurrency/crash tests; paid-session end-to-end sandbox testing;
real HTTPS receiver delivery testing; unknown/expired checkout recovery; a trusted
reconciliation path for previously paid operations whose product grant is revoked;
refund/dispute event handling; outbox monitoring/manual recovery; and live rollout
gates. No public endpoints/scheduler are wired, no DB migration is committed, and
SEYFR/Gelato fulfilment remains disconnected. Existing invoicing still uses its
original workflow and keys through the extracted shared Stripe transport.

## Next slices, in order

1. Define a product-independent payment service contract and TypeDB schema/query
   repository. Model application registrations, merchant grants, payment requests,
   attempts, provider bindings, observations, refunds and outbox events. Use exact
   integer minor amounts with explicit currency/exponent; do not generalize USD by
   simply relabelling it. Validate provider-specific currency support separately.
2. Enforce merchant/application/environment authorization in every query. Product
   identity must come from authenticated, stored grants, never a trusted body field.
   Uniqueness scopes must include merchant, product and environment. Match existing
   request fingerprints on retry; changed payload with same operation is a conflict.
3. In a write transaction authorize and claim an operation before provider I/O.
   Persist verified financial state and its integration event atomically. Never hold
   a database transaction open across provider network calls. Snapshot isolation is
   not automatically serializability: use conflict-bearing records/unique keys and
   test races against the actual server version before claiming concurrency safety.
4. Add hosted one-time checkout. Distinguish setup from payment and delayed payment
   completion. Verify provider amount/currency/customer/reference/environment. Never
   mark paid from a browser return. Unknown provider outcomes require reconciliation.
5. Add authenticated outbox delivery with stable event IDs, acknowledgements,
   exponential backoff, dead-letter handling and replay. Consumers deduplicate and
   validate bindings. A duplicate payment event must not print a second product.
6. Migrate invoicing behind this contract with existing IDs, idempotency keys,
   scheduler semantics and consent intact. Avoid dual writes/two payment authorities.
7. Integrate SEYFR as a second consumer: guest checkout, immutable quote/order,
   confirmed payment -> durable fulfilment job -> Gelato. Booking gets its own
   adapter rather than invoice-specific fields in the shared core.
8. Complete sandbox end-to-end and failure tests, production restore/monitoring,
   gradual rollout and rollback before enabling live consumers.

## TypeDB responsibilities

Use schema constraints for permitted types, cardinality and uniqueness; TypeQL
queries/functions for reusable eligibility and relationship checks; transactions
for authorized claims and state transitions. Financial invariants belong in this
query boundary rather than duplicated browser/application predicates. Test schema
and query compatibility against the deployed TypeDB version (driver pinned to
3.13.6 does not prove the server version).

Provider HTTP, cryptographic verification, secret handling, asynchronous retries,
email and Gelato printing remain application/integration responsibilities. TypeDB
cannot atomically commit an external card charge or guarantee exactly-once network
effects. Outbox and reconciliation are mandatory, not optional because of the DB.

## Acceptance gates

- Existing invoicing/consent/portal regressions pass unchanged.
- Two independent consumers can use one-time payment without invoice/plan coupling.
- Cross-merchant/product/environment access and forged events fail.
- Concurrent same-key requests create one durable operation; changed payload fails.
- Provider timeout, crash, duplicate/out-of-order events and stale observations
  neither duplicate charges nor regress verified state.
- Partial/full refunds and disputes reconcile without deleting financial history.
- No live provider keys or customer data are used in default tests.

References: https://typedb.com/docs/core-concepts/typedb/transactions/
and https://typedb.com/docs/core-concepts/typeql/schema-data/ .
