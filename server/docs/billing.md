# Business billing — implementation and validation

Work in progress, 2026-10-08. **Sandbox integration, not approved for live customer billing.** Existing Render deployment has not been changed. No live payment keys were obtained, no real payments made, and no paid plan selected.

## Product behavior

- Seller company owners create billing accounts linked to an existing buyer company, then USD schedules: once, weekly, monthly, yearly; UTC invoice dates and 0–90 days to pay.
- Seller policy is manual, automatic, or customer choice. Automatic policy never manufactures consent. Buyer company owners choose their preference and review schedules before hosted Stripe card setup.
- Manual invoices can use Mercury (preferred) or Stripe. Automatic collection uses Stripe, with card data entered on Stripe's hosted page. The API rejects raw card fields.
- Customer consent is tied to a billing revision. Changes to schedules or preferences invalidate it. Verified SetupIntent evidence retains the actor, revision and timestamp. Stripe's customer, session, intent, card ownership, mode and metadata must agree before saving consent.
- Scheduler atomically advances the plan and creates its invoice. Concurrent triggers cannot generate another occurrence of the same plan sequence.
- Stripe invoices have `auto_advance=false`. Business explicitly attempts payment at the due date only if the invoice's consent revision remains current. The durable claim is written before the external request. Failed or uncertain attempts are never automatically charged again. Customers receive the hosted invoice link to complete payment or use another card. A payment already in flight can complete after a preference changes.
- Stripe webhook signatures are verified over the raw body, including timestamp tolerance. Payment status is retrieved from Stripe and checked against the bound customer, currency, invoice and amount. Unverified browser redirects cannot mark an invoice paid. Duplicate deliveries reconcile the current state.
- Mercury invoice status is polled. A transaction webhook alone is not evidence that a particular invoice was paid. The current Mercury invoice API does not provide the saved-card autopay workflow implemented here.
- Resend adapter prepares plain-text invoice notices, payment acknowledgments and overdue reminders. Default reminder interval is seven days, capped at four overdue reminders per invoice. Each notice has a durable unique key. Unknown delivery outcomes remain `attention` for reconciliation, rather than blindly resending after provider idempotency expires. `accepted` means accepted by the email API, not delivery to the recipient's inbox.

## Costs and provider constraints

There is no fixed monthly subscription selected by this implementation. Payment processing and product usage charges can still apply. Mercury's current pricing table includes programmatic invoicing in its $0 plan; receiving ACH debits is a separate paid-plan capability and is disabled in requests. Card acceptance through Mercury requires its Stripe connection. Mercury API invoices use `SendNow` on their scheduled creation date.

Stripe charges processing fees and can charge Invoicing/Billing usage fees. Do not equate $0 setup/monthly commitment with fee-free transactions. Resend has a free tier with sending limits; no automatic paid upgrade is implemented or authorized. Sending is disabled until an account, verified sender and free-tier limits are confirmed.

Primary references reviewed:
- [Mercury pricing](https://mercury.com/pricing)
- [Mercury invoicing](https://docs.mercury.com/docs/invoicing)
- [Create Mercury invoice](https://docs.mercury.com/reference/createinvoice)
- [Get Mercury invoice](https://docs.mercury.com/reference/getinvoice)
- [Mercury sandbox](https://docs.mercury.com/docs/using-mercury-sandbox)
- [Stripe Checkout setup](https://docs.stripe.com/api/checkout/sessions/create)
- [Stripe invoice creation](https://docs.stripe.com/api/invoices/create)
- [Stripe explicit invoice payment](https://docs.stripe.com/api/invoices/pay)
- [Stripe webhooks](https://docs.stripe.com/webhooks)
- [Stripe test values](https://docs.stripe.com/testing)
- [Stripe pricing](https://stripe.com/pricing)
- [Resend email API](https://resend.com/docs/api-reference/emails/send-email)
- [Resend idempotency retention](https://resend.com/docs/dashboard/emails/idempotency-keys)
- [Resend free-tier limits](https://resend.com/pricing)

## Routes and configuration

`/billing` is the basic same-origin customer/seller UI. It keeps bearer tokens in memory; leaving the page requires signing in again. For jitpomi.com, deploy behind a same-origin reverse proxy or integrate the UI with its authenticated API client; do not introduce wildcard credentialed CORS.

Protected DogRS services: `/billing-customers`, `/billing-plans`, `/billing-invoices`, `/billing-actions`. TypeQL policy functions check current owner membership in the seller or linked buyer company. Invoices are read-only to API callers. Provider keys serve only `BILLING_MERCHANT_ID`; this is JITPOMI billing its customers, not a payment marketplace using one secret for arbitrary sellers.

`BILLING_ENABLED=true` enables `/internal/billing/tick` and `/webhooks/stripe`. The tick endpoint requires its own long random secret and allows one active invocation per process. TypeDB claims also protect across processes. The supplied Cloudflare scheduled Worker invokes it every five minutes; it is prepared but **not deployed**. Render free-host sleep/cold starts mean dates are scheduling intentions, not a precise delivery-time SLA. A tick has a 50-second deadline; unfinished claims require reconciliation.

Apply `database migrate-billing` only to `business_dev` first. The command supports repeating schema definitions and redefining functions atomically. The migration is not run implicitly at server startup.

See `.env.example` for variable names. Test keys belong in a protected file outside the repository (`0600`), never in commands, logs or screenshots. `BILLING_TEST_ENV` points the ignored provider tests to that file. Sandbox email additionally requires a matching `BILLING_TEST_EMAIL` recipient.

## Validation commands

```sh
cargo test --locked --lib
cargo clippy --locked --all-targets -- -D warnings
# Set BUSINESS_ENV_FILE to the protected non-admin business_dev connection.
cargo run --locked --bin database -- migrate-billing
cargo test --locked --lib hosted_billing_isolation_and_schedule -- --ignored --nocapture
# Set BILLING_TEST_ENV to the protected sandbox provider configuration.
BILLING_PUBLIC_URL=https://your-test-host.example/billing cargo test --locked --lib stripe_sandbox_invoice_lifecycle -- --ignored --nocapture
cargo test --locked --lib mercury_sandbox_invoice_lifecycle -- --ignored --nocapture
BILLING_PROVIDER_TEST=true cargo test --locked --lib hosted_billing_isolation_and_schedule -- --ignored --nocapture
```

The combined database/provider test inserts a trusted synthetic saved-card fixture after verifying a Stripe test SetupIntent. It verifies scheduling, concurrent collection and paid reconciliation; **it is not proof that the customer's browser setup/return journey has passed**.

## Remaining launch gates

- Browser-to-Checkout-to-Business consent confirmation, decline/expired-card/3DS cases, and exact consent wording need full acceptance testing.
- Deploy the validated build and scheduler, register the real sandbox webhook endpoint, then replay signed webhook deliveries through the public HTTPS path.
- Create/verify the free email sender and test actual delivery, bounce handling and free-tier exhaustion. No real customer email is enabled yet.
- Test recovery of each uncertain external write. A saved draft ID is retained before item/finalize calls; failures are stopped for operator reconciliation. Unknown create outcomes must be located by metadata/invoice number. Never create a replacement invoice just because a request timed out. There is not yet an operator recovery UI.
- Verify Mercury hosted invoice URL and real card acceptance/Stripe connection in the sandbox. `Paid` can be an out-of-band mark, so the acknowledgment says the invoice is marked paid; it is not a bank-settlement certificate.
- Decide and implement the desired catch-up policy after prolonged downtime or a paused recurring schedule. Current scheduling catches up due occurrences in bounded batches; do not enable it for live billing without explicit acceptance of that behavior.
- Review taxes, invoice legal identity, cancellation/refund/dispute handling, pagination beyond 100 UI records and operational alerting before live use.

No claim of full production readiness is made by passing these initial sandbox checks.

### Email validation — 2026-10-08

The existing JITPOMI Resend account is on its Free plan; the dashboard showed
3,000 monthly / 100 daily transactional emails and paid overages disabled.
`jitpomi.com` is verified. A dedicated sending-only key restricted to that domain
is stored only in the protected local test configuration. The unused initial key
was deleted and its replacement was verified on disk before closing its display.

`resend_test_delivery_and_idempotency` passed against the real Resend API.
A clearly marked test message was sent only to the approved `dev@jitpomi.com`
recipient. Resend's dashboard reported **Delivered**. Repeating the same request
with the same idempotency key returned the same message ID. This validates provider
submission and delivery of that test email, not the complete scheduled receipt /
reminder workflow. Automatic mail remains disabled by default and no deployed
service configuration was changed. Mercury initial invoice notices are suppressed
in the custom mail path because Mercury's `SendNow` already sends them.

### Scheduled payment and email flows — 2026-10-08

`hosted_billing_payment_and_mail_flows` passed against real `business_dev`, Stripe
sandbox and Resend. New synthetic seller/buyer owners created schedules through
protected application routes. Both were one-off USD 12.50 invoices due on issue.
The test asserted no invoice before its schedule, ran the actual `Engine::tick`,
and checked both provider state and the persisted invoice/notification records.

| Scenario | Stripe invoice | Final state | Email evidence |
| --- | --- | --- | --- |
| Successful saved-card payment | `in_1UONaDCs4HOI1uJlywNm7Dgs` | paid, USD 12.50 paid | receipt `01a11d1f-f432-72e7-b9c4-819c17496eb1`, Delivered |
| Card declined after successful setup | `in_1UONaPCs4HOI1uJlITTp171V` | open, USD 0 paid, attempted | reminder `01a11d20-2317-756f-9281-254ec767809f`, Delivered |

Both emails were marked TEST and sent only to `dev@jitpomi.com`; delivery was
verified in Resend's dashboard. Repeating the scheduler yielded zero new invoices,
zero notices, zero operation errors and unchanged Stripe payment-attempt counts.
Fixtures used `pm_card_visa` and `pm_card_chargeCustomerFail` in Stripe sandbox.
No real funds moved.

Scope: saved-card consent was inserted as a trusted test fixture after a successful
sandbox SetupIntent. This test does not certify interactive Checkout consent, a
public webhook delivery, deployment of the Cloudflare cron, or live billing.
Scheduling was invoked locally against Cloud data, using actual due timestamps;
the production hosting configuration remains unchanged. Each flow sent its real
test email through the application's notification code, not a separate mail script.

Run explicitly (private env paths supplied by the operator):

```sh
BUSINESS_ENV_FILE=/path/to/database.env \
BILLING_TEST_ENV=/path/to/approved-sandbox.env \
BILLING_MAIL_ENABLED=true \
cargo test --locked --lib hosted_billing_payment_and_mail_flows -- --ignored --nocapture
```

A second run passed in 29.55 seconds after adding an explicit assertion that the
buyer-facing protected invoice list reports the same status. It created sandbox
invoices `in_1UONbICs4HOI1uJlCbHD2rv9` (paid) and
`in_1UONbTCs4HOI1uJlbHD07pTl` (declined/open). Clippy with warnings denied passed.
The additional two test emails belong to this separate repeat run; scheduler
replays within each run sent no duplicates.

### Customer portal and onboarding — implementation progress

The companion website checkout now has `portal/`, a Cloudflare Worker and static
`/account/` interface. New migration 006 stores hashed, expiring one-use invitations.
`/onboarding` invitation creation requires the configured merchant's current owner;
acceptance creates a new identity or verifies an existing password before granting
the invited company's owner membership. The invitation and membership commit in one
transaction. Existing user passwords are never replaced by invitation acceptance.
`hosted_customer_onboarding` passed on Cloud business_dev for new and existing users,
replay rejection, merchant-only creation and cross-tenant rejection.

The backend can require `PORTAL_ORIGIN_SECRET` to stop ordinary requests bypassing
the portal's session/edge protections. Health and signed billing ingress retain their
own controls. Billing lists accept a validated `after` UUID cursor. The frontend has
customer-only history/preferences and merchant-only invitation/schedule/policy controls.
Local browser checks confirmed seller login/logout and a buyer seeing their paid
sandbox invoice, with no administrative controls. The worker keeps backend tokens
in an encrypted Secure/HttpOnly/SameSite cookie rather than browser-readable storage.

The user approved hosted sandbox credential transfer. The free Render validation
service now runs the billing branch, and the companion portal is deployed at
https://jitpomi.com/account/. Cloudflare runs the authenticated scheduler every
five minutes. HTTPS buyer login, logout, and paid invoice history were verified.
A signed sandbox webhook probe returned HTTP 200 after the environment deployment;
this probe is not proof of Stripe delivery. A hosted scheduler invocation returned
HTTP 200 with zero errors and one receipt notification after correcting the
synthetic customer billing email to the approved dev@jitpomi.com recipient.

Browser-driven card setup/return also passed: the customer reviewed the schedule,
saved Stripe test card 4242 in hosted Checkout, returned to jitpomi.com, and received
the verified-authorization confirmation. The future test schedule was paused afterward.

The scheduled USD 1.00 sandbox invoice `in_1UOOOZCs4HOI1uJlwxWR9Fc2` was issued
through the hosted tick and paid with an attached sandbox card via Stripe test API.
Before another explicit reconciliation, the protected customer API showed paid.
Stripe event `evt_1UOOOzCs4HOI1uJlt6lHWalo` reported zero pending webhooks.
A receipt tick sent one notice with zero errors; the replay sent zero notices.
The older fixture first blocked mail to example.com and then queued a Mercury invoice
because no hosted Mercury credential exists. Correcting its billing recipient to
dev@jitpomi.com and its manual provider to Stripe resolved both, without weakening
the test-email restriction or copying the IP-restricted Mercury token.
This is a sandbox deployment, not live customer billing. The website portal README
tracks remaining launch requirements. Do not import real customers or activate live
collection until those requirements are satisfied.


### Customer-launch audit, 2026-10-08

Implemented and tested against Cloud business_dev:

- Card setup requires the revision of the schedules actually reviewed. The portal
  loads every customer and schedule page, reads customer revisions before plans,
  and freezes the reviewed revision before opening its consent dialog. A concurrent
  change rejects Checkout authorization; a redirect alone still cannot grant consent.
- Plan creation requires a caller-generated UUID `request_id`. A retry with the same
  ID and immutable terms returns the original plan; different terms are rejected.
  The UI retains the request ID for failed attempts within the current page session.
- Merchant owners can revoke or replace an unused invitation. Rotation immediately
  invalidates the old token. Accepted invitations cannot create another membership.
- Merchant-only `reconcile_invoice` takes a local invoice ID and provider invoice ID.
  It observes an existing provider invoice and verifies customer, amount, currency,
  provider and local metadata binding; it neither creates invoices nor charges cards.
  Authorization is rechecked in the write transaction. Drafts with incomplete amounts
  must be investigated in the provider dashboard; do not recreate uncertain invoices.
- Notification failures are counted individually. Invoice attempts rotate through
  oldest work even on provider error/cancellation, reducing batch starvation.
- Live startup requires separate `business_prod`, distinct origin/scheduler secrets,
  a live Stripe key and webhook configuration. This is configuration validation,
  not evidence that production provisioning or live activation has happened.

Evidence from this audit: stale-consent, invitation rotation/revocation, schedule
retry/isolation and operator reconciliation regressions passed on real TypeDB Cloud.
Stripe sandbox payment/email flow passed in 30.98 seconds: invoice
`in_1UOOdbCs4HOI1uJlMP8vWeLo` paid and receipt accepted; invoice
`in_1UOOdoCs4HOI1uJlkUq7sPmj` declined and reminder accepted. Replays created no
new notices or payment attempts. Email acceptance is not inbox-delivery certification.
These tests used current local code, sandbox payments, and only dev@jitpomi.com.

Remaining launch blockers: production database/merchant bootstrap and migration,
secure customer password recovery, explicit refunds/disputes/partial-payment history,
operator handling of uncertain mail delivery, and an exercised production restore
and support procedure. Free-host cold starts also affect availability. Keep
`BILLING_LIVE=false`; these audit fixes do not constitute live-launch approval.
