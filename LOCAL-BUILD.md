# Local safety build

This branch keeps the existing Windows application and adds process ownership, filesystem confinement, read-only diagnostics and error/status handling. It does not add detection-evasion behavior or make Quest spoofing permitted or account-safe. Upstream's unfinished rewrite is not included.

## Build

`.github/workflows/safety-build.yml` builds the Windows runner and application from this branch's source. It runs frontend regression tests, native safety tests and a portable startup check before uploading an artifact named with the exact source commit. The workflow does not create a GitHub release or publish anything upstream.

The portable folder includes `build-info.json` and `SHA256SUMS.txt` so a downloaded build can be checked against its source revision. The local build uses a separate app-data identifier and window title. It reuses the existing package version rather than creating a new release version or tag.

For a source build, install Node.js, Rust and the Windows MSVC/SDK prerequisites, then run these commands in PowerShell from the repository root:

```powershell
npx --yes pnpm@8.15.3 install --frozen-lockfile --ignore-scripts
node scripts/prepare-catalog.mjs
npx --yes pnpm@8.15.3 test
npx --yes pnpm@8.15.3 build
cargo build --release --locked --manifest-path src-win/Cargo.toml
Copy-Item src-win/target/release/src-win.exe src-tauri/resources/src-win.exe
cargo test --release --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1
npx --yes pnpm@8.15.3 exec tauri build --ci --no-bundle --config src-tauri/tauri.local-safety.conf.json -- --locked
```

The generated fallback catalog comes from a pinned upstream revision and is checked against an expected SHA-256, rather than accepting an unvalidated build-time download. It is not committed as a second large data snapshot.

## Changed behavior

- **Stop** targets only a retained child-process handle owned by this app, not every process sharing a filename. The app observes natural exits and stops its owned children when exiting.
- **Filesystem paths** reject traversal, reserved names and symlink/junction ancestors. Copy does not overwrite unrelated files, and launch accepts only content matching the packaged runner.
- **Playground diagnostics** check Discord IPC availability without publishing activity or changing a connection. Availability is not authentication or an active Rich Presence connection.
- **RPC** keeps the existing explicit Home confirmation. Connection/disconnection operations are serialized, bounded and awaited. Debug output no longer dumps the account's user object.
- **Catalog fetching** checks HTTP errors, parses JSON, validates all entries and moves to the next source when a response is invalid. A successful retry clears the previous error.
- **Status and logs** reflect successful operations and reactive updates rather than optimistic flags. Failed observations do not falsely mark a process stopped.

Native tests use disposable local process/filesystem fixtures and a local HTTP server. Frontend tests use real Vue/VueUse with mocked native commands. Neither suite performs Quest actions or publishes live Discord activity.

## Installation and rollback

Keep each verified build in a separate directory with its `data` and `resources` folders. Do not copy over a running executable. A staged local build can sit beside the original installation; closing the old app and starting the local shortcut switches versions. Keep the original folder/shortcut for rollback. The app's safety checks are not a guarantee against all same-user filesystem races or against Discord enforcement.
