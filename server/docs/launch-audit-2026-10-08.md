# Business customer-launch audit — 2026-10-08

## Conclusion

The exercised sandbox flows pass. This is not a live-launch certification. The
Mercury integration remains blocked by sandbox OAuth and payment mapping questions
under support case **2717152**. No live payments or paid infrastructure were used.

## Fixes in this audit

- Stripe card setup previously reused a customer/terms/actor idempotency key. A
  real Stripe sandbox test created a session, expired it, and retried unchanged
  terms: the old code returned the same expired session. Each new setup interaction
  now uses its own request key. The same test passes with a fresh open session;
  both test sessions are expired afterward. Setup cannot charge a card. Invoice
  and charge idempotency keys are unchanged. Consent still verifies the customer,
  actor, SetupIntent, attached card and current terms revision before committing.
- Portal JSON parsing now rejects malformed/null/array input with 400, unsupported
  content type with 415, and bodies above 16 KiB with 413. These used to appear as
  503 outages. Regression tests verify that no backend request occurs.
- Local Cloud-backed acceptance tests now send the configured portal-origin secret.
  The onboarding test initially failed with missing fixture configuration, then
  with 401 because its helper omitted that header. The harness was fixed without
  disabling application protection, and onboarding then passed.

## Evidence collected

| Check | Result and limits |
|---|---|
| Rust unit suite | 15 passed; 25 hosted/diagnostic tests explicitly ignored by the default suite |
| Rust formatting and Clippy | Passed, including all targets and warnings denied |
| Portal Worker | 12 tests passed |
| Scheduler Worker | 2 tests passed |
| Scheduled invoice → payment → receipt | Cloud business_dev + Stripe sandbox + Resend passed; replay made no duplicate attempt or notice |
| Declined payment → reminder | Same real services passed; replay did not recharge |
| Password recovery | Cloud passed generic response, durable cooldown, concurrent single consumption, old-link/session/password rejection and fresh login |
| Customer onboarding | Cloud passed new/existing identity acceptance, single-use invitation, merchant-only creation and cross-tenant rejection |
| Stale consent | Cloud rejected outdated revision before creating Checkout |
| Operator invoice reconciliation | Cloud + Stripe sandbox passed invoice binding and owner-only access checks |
| Partial payment/refund/dispute history | Real Stripe sandbox passed |
| Expired Checkout retry | Failed before the fix; passed afterward against Stripe sandbox |
| Existing public HTTPS deployment | Login/logout, buyer scope, invoice history, Stripe payment details, stale/missing consent rejection, operator-only recovery denial and password recovery rejection checks passed |

Payment flow evidence: successful invoice `in_1UOTNNCs4HOI1uJlKYSdAs0m`;
declined invoice `in_1UOTNZCs4HOI1uJl1z2OvK2K`. Emails went only to the approved
`dev@jitpomi.com` address. The API accepted both messages; inbox delivery was not
independently inspected in this audit. Tests use synthetic identities and a trusted
saved-card fixture; they do not replace a complete browser/3DS acceptance test.

The new fixes are source changes validated locally with real sandbox services.
The public HTTPS canary exercised the pre-existing deployment, not these new fixes.
Deploy the reviewed backend and portal revisions in sandbox and repeat the canary
before treating them as deployed. No production database was provisioned here.

## Remaining gates before real customers

1. Deploy these fixes in sandbox and finish browser acceptance for card replacement,
   completed-session retries, authentication-required payments, expired cards,
   invitation replacement/revocation and recovery email links.
2. Provision the separate business_prod database and real merchant owner; apply
   migrations, protect runtime credentials, and validate that deployment over HTTPS.
   This requires the deliberate live-provider configuration step. Never copy
   synthetic users, consent or pending payment claims into production.
3. Resolve Mercury case 2717152 before promising Mercury card-payment history or
   sandbox card acceptance. Until then, qualify launch as Stripe-only if chosen.
4. Exercise uncertain create/finalize/send outcomes and operator recovery end to end.
   Durable claims stop blind retries; they do not automatically resolve every
   interrupted operation. Add actionable monitoring for stuck claims and failed ticks.
5. Establish scheduled encrypted backups outside the application host and rehearse
   restoration of the production deployment. Prior same-cluster validation restore
   tests do not certify region loss, a backup schedule or recovery time.
6. Explicitly approve the recurring schedule catch-up behavior after pauses/outages,
   and verify customer-facing invoice identity, receipt/refund handling and mail
   bounce/quota procedures. Free Render cold starts preclude a precise timing SLA.

## Provider references

[Stripe Checkout creation](https://docs.stripe.com/api/checkout/sessions/create)
documents session expiry (24 hours by default).
[Stripe idempotency](https://docs.stripe.com/api/idempotent_requests)
explains why reusing a key returns the original response.
[Stripe invoice payment](https://docs.stripe.com/api/invoices/pay)
describes the explicit payment operation, separate from card setup.
