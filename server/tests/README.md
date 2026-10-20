# Backend tests

Tests are separate from source, following the DogRS/sp-connector service layout.

- `backend/`: real TypeDB policy and main-server HTTP tests, plus executable provisioning/bootstrap/login/data/logout acceptance. Uses the same app composition as production; no alternate server.
- `mercury/`, `stripe/`: registered resource/auth hooks, tenant boundaries, HTTP transport and separately gated real sandbox checks.
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
