# Business server

This application uses published DogRS 0.3.0 crates, dog-transport HTTP hosted by
Axum, and TypeDB 3.13.6 (the driver version required by dog-typedb).

Development and production use the same code, TLS validation, auth strategy and
durable TypeDB token store. Set `BUSINESS_ENV_FILE` to an owner-readable file
outside the project. The prepared configurations are in
`~/.config/jitpomi/business/dev.env` and `prod.env`. Their credentials must not be
committed or pasted into logs. Use a dedicated non-admin runtime account.

```sh
BUSINESS_ENV_FILE="$HOME/.config/jitpomi/business/dev.env" cargo run --locked
```

The HTTP server binds to the configured interface (prepared configs use loopback).
`/health` is public. `/authentication` accepts local email/password authentication
with `x-tenant-id` naming a stable organization UUID (or current domain alias); the resolver verifies the user's actual
tenant membership. Passwords stored in TypeDB must be bcrypt hashes. No registration
or calendar/invoice endpoints are implemented yet. The legacy raw `/subjects`
service is no longer registered, including for internal service calls.

TLS is mandatory and AUTH_JWT_SECRET must contain at least 32 bytes. Database
creation/schema loading is explicit via the database binary; normal startup never
creates, resets, or deletes a database. TYPEDB_FORCE_RECREATE is rejected.

The free hosted cluster pauses after three inactive days and provides no managed
backups. Development and production databases share cluster resources. TypeDB's
current standard-user model allows access to all databases, so their separation is
not an enforced database credential boundary. Do not expose credentials to browser
clients. A dedicated cluster is needed for stronger environment isolation.

Tests: `cargo test --locked`. The ignored `hosted_auth_and_revocation` test only
permits `business_dev`, creates uniquely named synthetic fixtures, and checks login,
wrong-tenant rejection, redaction, logout, public query blocking and concurrent
refresh consumption against the real cloud. It retains synthetic fixtures for
inspection. Do not point tests at production.

Verified on the real free TypeDB Cloud cluster: schema/functions loaded for both
Business databases; development login, wrong-tenant rejection for local login and JWT reuse, password redaction,
logout/revocation, blocked raw queries, and concurrent refresh consumption pass.

Remaining release work includes authorization for additional domain services, abuse/rate limiting,
backup/export procedures and hosted HTTP deployment. This
foundation migration alone is not a production-readiness certification. Both dev
and prod configurations passed cloud-backed startup and HTTP health checks; neither
HTTP server is left running or publicly exposed.

## Tenant-scoped private records

`/records` is a deliberately small business-data foundation, not a calendar or
invoicing implementation. Every operation requires a valid bearer JWT plus
`x-tenant-id`. The selected tenant, current membership and verified creator scope
all reads and writes. A user belonging to A and B must select A to access their A
records; their B membership cannot bypass that boundary. Records are private to
the creator even within one tenant. Team sharing, product entitlements and project action permissions are implemented
separately by the workspace APIs described below; they do not widen this private API.

- `POST /records` accepts only `{ "name": "Example" }`; IDs, tenant and creator
  are assigned by the server. Unknown fields are rejected.
- `GET /records` returns at most 100 own records in the selected tenant, sorted
  by ID. There is no pagination/filter API yet.
- `GET`, `PUT`, `PATCH`, `DELETE /records/{uuid}` only act on that same scope.
  PUT/PATCH accept the same name-only input. Bulk updates/deletes are unsupported.
- Names currently allow 1–128 ASCII letters/numbers, spaces and `.,_-()`.
- A foreign record and a nonexistent record both return 404. Missing/invalid
  credentials or revoked membership return 401. Internal callers must also supply
  a valid token; setting an authenticated flag does not bypass validation.

The identity is verified before database access, and membership is also matched
inside the same TypeDB transaction as the data operation. Revocations apply to
subsequent requests; already-running transactions use their database snapshot.
Database credentials remain privileged and must never be given to clients.

For an existing database, apply the additive schema with `database migrate-records`
using the provisioning account and explicit `TYPEDB_DB`. This does not reload IAM
functions or recreate the database. Fresh provisioning includes these types in
`schema.tql`. Normal application startup does not migrate schema.

Run both hosted suites on the real free development database:

```sh
BUSINESS_ENV_FILE="$HOME/.config/jitpomi/business/dev.env" \
  cargo test --locked -- --include-ignored --test-threads=1
```

`hosted_business_data_isolation` exercises the actual Axum/DogRS HTTP router with
unique synthetic identities and tenants: cross-tenant list/get/update/patch/delete,
a user belonging to both tenants, same-tenant creator isolation, forged ownership
fields, valid own-record CRUD, membership removal and logout. It also checks that
internal calls cannot forge authentication. It refuses any database name other
than `business_dev` and retains synthetic fixtures for inspection.

These tests cover this records API, not every legacy IAM function or future
calendar, invoice, app-entitlement or project-sharing endpoint. New domain services
must enforce and test their own selected-tenant and action permissions.

## TypeDB design reference

See [TypeDB foundation](docs/typedb-foundation.md) for the JITPOMI umbrella model,
source-backed constraints and next implementation sequence.
[Reading coverage](docs/typedb-reading-coverage.md) distinguishes reviewed material
from the remaining documentation.

## Shared organization and product access

See [Workspace access implementation](docs/workspace-access.md) for the new TypeQL policies, app/project discovery, shared-record endpoints, explicit development migration and live test command. Apply the access migration before running this server revision against an existing database. Existing private records remain private.

## Identity and concurrency hardening

[Audit and validation](docs/identity-concurrency-audit.md) covers stable UUID token subjects, canonical memberships, owner-only administration and coordinated writes. Apply the development access migration before using this revision; existing email-subject tokens require a new login.

## Deployment validation

See [current deployment validation evidence](docs/production-validation-progress.md) for recovery, restore, policy-query measurements and the remaining deployment/load gates. The Render blueprint uses only the free plan. Do not infer full production validation from the correctness tests alone.

The ignored hosted HTTP suites can target the validation deployment by setting
`BUSINESS_TEST_BASE_URL=https://jitpomi-business-validation.onrender.com` alongside
an explicitly selected `BUSINESS_ENV_FILE` for `business_dev`. Their synthetic
fixtures are still provisioned directly in that development database; HTTP calls
then use the public endpoint. The local JWT configuration must match the tested
service for the suites that also check internal-service authentication. Never
point this harness at a production database. Secrets are not command arguments.

`public_restart_prepare` saves a synthetic record/session fixture at a new path
specified by `BUSINESS_RESTART_FIXTURE` with mode 0600. Request exactly one Render
restart, wait for a new process start in provider logs and healthy HTTPS, then run
`public_restart_verify` with the same private fixture. It verifies persisted data
and logout, then removes its record and logs out the remaining token. A failed
mutation response can mean the write committed: inspect the saved resource before
retrying; this harness does not automatically retry writes. Do not commit fixture
files or place them in the public repository.
