# Business deployment validation — 2026-10-08

Status: **not yet production-validated**. Validation uses the existing free TypeDB Cloud `business_dev` database. Production data is not part of these tests.

## Completed evidence

- A live connection-loss/transaction-timeout test passed (11.89 seconds). Buffered changes remained absent after timeout, disconnect before commit, and the observed commit/disconnect race; a fresh connection could write afterward. The race did not demonstrate a committed transaction with a lost acknowledgement.
- A backup/restore drill exported `business_dev`, restored it into a temporary database on the same cluster, compared 985 selected identity, relationship and policy rows, and removed the restored database. It passed in 113.83 seconds. This snapshot preceded the grant-function split below.
- Render's new-service form uses the Free plan ($0, 0.1 CPU, 512 MB). With explicit approval, ten environment variables were imported from a protected local file, including non-admin TypeDB credentials and a fresh JWT signing secret. The form has not been deployed. The TypeDB runtime account is not limited by the provider to this database.

## Performance investigation

The provisional acceptance workload is ten active tenants, ten concurrent users, five minutes of mixed protected reads/writes. The test uses the real TypeDB Cloud database through the in-process HTTP router. It requires at least 1,000 requests, p95 <= 5 seconds, p99 <= 10 seconds and no unexpected status codes. This is not a public-network load test.

Earlier attempts failed with 408 responses. Bounding active requests at two and waiting requests at 32 prevents unbounded demand, but by itself yielded 503 overload responses and did not fix slow queries. No threshold was relaxed to turn those failures into passes.

Grant evaluation originally combined user and team grantees in one disjunction. A sequential diagnostic timed out after ten seconds on a grant check. A later idle diagnostic completed the full authorization check in 8,537 ms. After splitting direct and team grants into separate functions and successfully migrating the development database, the same diagnostic completed authorization in 302 ms (identity 78 ms; membership 85 ms; entitlement 73 ms; allow 272 ms; deny 64 ms). These are individual samples, not a capacity certification or proof that all latency came from that expression. The first migration attempt timed out opening a schema transaction; the retry succeeded.

## Remaining gates

- Correctness regression after the policy-function changes: **passed**, 13 tests in 101.60 seconds, including hosted auth/revocation, tenant isolation, connection recovery, identity concurrency, invalid migration rejection, protected-write atomicity and workspace policies. Clippy passed with warnings denied.
- Unchanged mixed-workload acceptance test: **passed**. Ten tenants and ten concurrent users completed 230 cycles / 1,610 requests in 307.60 seconds, zero unexpected responses, p95 2,181 ms, p99 2,218 ms, maximum 2,271 ms. This used the in-process router and live TypeDB Cloud, not a deployed HTTP endpoint.
- Review and publish a deployable source revision, then deploy the free validation service.
- Verify public HTTPS authentication, logout, tenant isolation, protected writes, restart behavior and operational limits against that deployed revision.
- Record exact results and distinguish tested recovery from untested provider failover, long network partitions and highly available operation. A free sleeping service does not establish an always-on availability guarantee.
