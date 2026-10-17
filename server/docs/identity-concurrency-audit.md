# Identity, uniqueness and concurrency audit

Date: 2026-10-08. Scope: current Business source, migration paths, policy functions and real development-database API behavior. This review checked invariants and adversarial counterexamples instead of treating the previous passing tests as proof. It is not external certification.

## Findings and fixes

| Finding | Risk | Fix and evidence |
|---|---|---|
| Email used as token subject and private-record ownership | Renaming or reusing an email could break ownership or transfer access. | Stable user UUID subjects; tenant UUID resolution; private owner IDs backfilled. Live tests rename email/domain, retain access with the same token and tenant UUID, then reuse the old email without transferring records. |
| Random membership keys | Two active relations could represent one pair, or an alternate active row could defeat suspension. | Canonical pair key enforced by creation and checked by policy. Two overlapping insert transactions produce one successful commit. An alternate-key active row does not restore access. |
| Legacy migration reruns | Deleted/revoked memberships could be recreated from old relations. | Persistent migration marker; rerun test leaves a removed canonical membership absent. Explicit owner action can re-enrol it. |
| Owner-controlled entitlement reactivation | An owner could restore access suspended by billing. | The public endpoint can only suspend entitlements; activation requires trusted provisioning. |
| Snapshot checks for last-owner protection | Two snapshots could each see another owner and both remove themselves. | Owner policy plus shared tenant revision write. Two overlapping owner-removal transactions result in exactly one commit; the remaining owner cannot remove themselves through the API. |
| Permission check followed by concurrent revocation | A stale authorized write could commit after a grant was revoked. | Protected writes and owner policy mutations participate in the tenant revision protocol. A revocation through HTTP commits; an older pending write fails and the original data remains intact. |
| Membership lifecycle not checked during JWT revalidation | A suspended canonical membership could remain authenticated for otherwise protected endpoints. | Local/JWT resolution invokes the canonical active-membership policy. These cases now return 401; downstream project denial remains 404. |
| Migration language and query composition defects | Backfill or administrative operations could fail before enforcing the intended rules. | Corrected explicit IID typing, attribute/value bindings, reused attributes and duplicate variable assignment; live migration and API tests exercise the corrected queries. |

## Test boundaries

The live suites use only synthetic records in `business_dev`. No production changes, account changes or paid services were performed. Direct database fixture writes establish controlled scenarios; public owner operations are tested separately. Tests retain fixtures for inspection, except deliberately malformed duplicate rows which are explicitly removed.

The concurrency tests open both database transactions before either commit. This establishes overlapping snapshots; merely launching two HTTP requests would not prove overlap. Separate HTTP assertions verify owner-only administration, self-escalation denial, protected access, revocation and last-owner behavior.

The coordination guarantee applies to supported writes that update the tenant revision. Privileged direct TypeQL administration cannot be sandboxed by these application policies. Out-of-band identity/product suspension and arbitrary direct grant writes need an operational protocol; no public endpoints for those global state changes are exposed here. Read snapshots and already-running logout behavior are unchanged. Same-tenant write contention is a deliberate tradeoff and has not been load-certified.

## Commands

```sh
BUSINESS_ENV_FILE="$HOME/.config/jitpomi/business/dev.env" cargo run --locked --bin database -- migrate-access
BUSINESS_ENV_FILE="$HOME/.config/jitpomi/business/dev.env" cargo test --locked -- --include-ignored --test-threads=1
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

Final validation passed: **9 tests, including 4 hosted suites**, zero failures, in 166.27 seconds. Clippy with warnings denied, formatting and diff checks passed. The explicit development migration also succeeded. Source fingerprints and scope limitations are recorded in [identity-concurrency-validation.json](identity-concurrency-validation.json).


## Additional hardening — 2026-10-08

A further source review found that the generic adapter committed writes before service-level result cardinality checks. Protected Business mutations now share a transaction executor that requires exactly one result **before commit**. Live tests buffer changes and then produce zero or multiple results, verify rejection, and read back unchanged data and tenant revision. A separate valid write proves the executor can commit successfully. This change is application-specific; no published DogRS crate was modified.

Write transaction opening, collection and commit acknowledgement have client bounds; the server also bounds the write transaction. Query failures are returned without exposing database query details. Unconfirmed commit outcomes return a conservative 503 and never trigger an automatic retry. Network loss at commit and timeout fault injection have not been certified by these tests.

Migration validation now rejects malformed/noncanonical user and tenant UUIDs and private ownership references with no matching identity. Live tests inject malformed data inside uncommitted transactions, assert migration refusal, close them and verify no malformed fixtures persisted. Authentication also rejects malformed stable identifiers.

This follows TypeDB's documented [snapshot isolation and commit conflict semantics](https://typedb.com/docs/core-concepts/typedb/transactions/). Tenant revision coordination remains necessary; the result checks do not turn snapshot isolation into general serializability.

The earlier nine-test result above is historical. The final hardening run passed **11 tests, including 6 hosted suites**, with zero failures or ignored tests, in 170.07 seconds. Clippy with warnings denied, formatting and diff checks passed. The development migration succeeded. Results and source hashes are recorded separately in [protected-write-validation.json](protected-write-validation.json).
