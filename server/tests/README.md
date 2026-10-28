# Backend tests

Tests are separate from source, following the DogRS/sp-connector service layout.

- `backend/`: real TypeDB policy and main-server HTTP tests, plus executable provisioning/bootstrap/login/data/logout acceptance. Uses the same app composition as production; no alternate server.
- `mercury/`, `stripe/`: registered resource/auth hooks, tenant boundaries, HTTP transport and separately gated real sandbox checks.
- `billing/`: canonical scheduler and real sandbox payment/receipt/reminder acceptance (explicitly gated); source-adjacent tests are linked from this directory.
- `unit/`: current configuration, database-tool validation, token keys, admission and data envelopes.
- `frontend/`: the actual frontend TypeQL client and binding contracts.
- `layout/`: every Rust source/test must be reachable from a Cargo target,
  and each service must have its common service/shared/hooks/schema modules.

Run `cargo test --locked --all-targets` and
`node --test tests/frontend/data-client.test.mjs`.
Real integrations are intentionally ignored by default, not empty placeholders:

```sh
BUSINESS_ENV_FILE=/private/dev.env BUSINESS_RUNTIME_ENV_FILE=/private/runtime.env cargo test --locked --test backend -- --ignored --test-threads=1
```

The first file supplies existing development-cluster operator credentials with
`TYPEDB_DB=business_dev`; the second supplies non-admin runtime credentials for
the same cluster. Tests create only generated disposable databases and never reset
configured databases. Provider checks must be selected separately; never run all
ignored tests with production credentials. A provider proxy test is not evidence
that recurring billing, onboarding, recovery or fulfillment workflows exist.

## Real-provider HTTP acceptance

`tests/acceptance/provider-http.mjs` exercises the deployed validation service or
an explicit loopback instance, using real owner login, tenant hooks and provider
HTTP. Supply `BUSINESS_TEST_BASE_URL`, `BUSINESS_TEST_TENANT`,
`BUSINESS_TEST_OWNER_EMAIL`, `BUSINESS_TEST_OWNER_PASSWORD` from a private
environment, and select `BUSINESS_TEST_PROVIDER=stripe` or `mercury`.

Stripe additionally requires `BUSINESS_ALLOW_PROVIDER_TEST_WRITES=1` and an
explicitly registered TEST connection. It verifies synthetic customer CRUD and
idempotency, unconfirmed setup/payment-intent cancellation, Checkout expiration,
draft invoice/item reads, cleanup, tenant denial and logout. It never confirms a
payment, finalizes an invoice or sends email. Mercury performs read-only checks of
all four resources and fails if upstream access is denied; an allowlist failure is
not reported as a passing end-to-end test. No provider token is sent by this test
client or printed in its output.
