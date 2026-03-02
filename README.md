# Business Server

A Rust-based business application server using TypeDB for data persistence.

## Environment Variables

### Required Variables

- `HTTP_HOST` - Host address for the HTTP server
- `HTTP_PORT` - Port number for the HTTP server
- `TYPEDB_ADDR` - TypeDB server address
- `TYPEDB_DB` - TypeDB database name
- `TYPEDB_USERNAME` - TypeDB username
- `TYPEDB_PASSWORD` - TypeDB password

### Optional Variables

- `TYPEDB_TLS` - Enable TLS for TypeDB connection (default: "false")
- `ENVIRONMENT` - Application environment (default: "development")
- `TYPEDB_FORCE_RECREATE` - Force database recreation (default: "false")
- `AUTH_JWT_SECRET` - JWT secret for authentication (default: "dev-secret")
- `AUTH_SERVICE` - Authentication service name (default: "accounts")
- `AUTH_ENTITY` - Authentication entity type (default: "user")
- `SUPER_COMPANY` - Super company identifier (default: "JITPOMI")

## Database Behavior

The server implements environment-aware database management:

- **Development mode** (`ENVIRONMENT=development`): Database is recreated on each restart for clean state
- **Production mode** (`ENVIRONMENT=production`): Existing database is preserved, only creates if missing
- **Force recreation**: Set `TYPEDB_FORCE_RECREATE=true` to force database recreation regardless of environment

This ensures data persistence in production while allowing clean development cycles.
