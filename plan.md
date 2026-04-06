# db2-node Plan

Last updated: 2026-04-06

## Snapshot

`db2-node` is now in release-prep mode rather than early prototyping.

Completed in the current local workspace:

- Pure Rust DRDA client, protocol crate, and Node.js bindings are in place
- Query timeouts, reconnect handling, pool health checks, and transaction cleanup are implemented
- TLS verification defaults are safe, system CA loading works, and Rust + Node TLS integration tests pass
- Prepared statements are isolated correctly per connection and no longer overwrite each other
- Node package metadata, exports, prebuild workflow, and docs site are all cleaned up
- Hugo docs now build locally, serve under the GitHub Pages subpath, and include basic search

## Release Checklist

Still worth doing before the first public GA tag:

1. Push a `v0.1.0-rc.1` or `v0.1.0` tag and verify `.github/workflows/release.yml` end to end.
2. Smoke test `npm install db2-node` on a clean machine with no local repo checkout.
3. Publish the docs update and confirm navigation + search on GitHub Pages.
4. Write a short changelog / release announcement for the initial package release.

## Current Verification

Useful commands for the final release pass:

```bash
cargo fmt --all --check
cargo check -p db2-client -p db2-proto -p db2-napi
cargo test -p db2-proto
DB2_TEST_SSL_PORT=50001 cargo test -p db2-integration-tests -- --nocapture
cd tests/node && DB2_TEST_SSL_PORT=50001 npm test
make docs-build
```

## Post-Release Roadmap

After the first stable release, the highest-value follow-up work is:

- Package iteration beyond the current per-connection prepared statement section budget
- Broader long-running soak / compatibility testing across more DB2 environments
- More polished application examples and deployment guides
- Better observability hooks, performance benchmarks, and profiling notes
