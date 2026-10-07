## Summary

<!-- One or two sentences on what this PR changes and why. -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / code quality
- [ ] Documentation
- [ ] CI / build
- [ ] Security fix
- [ ] Breaking change

## Checklist

### Code quality

- [ ] `cargo fmt --all` passes locally
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes locally
- [ ] `cargo test --workspace` passes locally (Linux + Windows if possible)
- [ ] No `unwrap()` / `expect()` in production paths - errors are propagated with typed errors
- [ ] No `unsafe` blocks (or justified with a comment if unavoidable)

### Shell integration

- [ ] If shell profile injection changed: verified idempotency (re-running `nvsn init --apply` is safe)
- [ ] If the profile block parser/writer changed: existing tests updated
- [ ] If a new shell is added: all four implementations (bash, zsh, fish, PowerShell) updated

### Release impact

- [ ] Commit message follows [Conventional Commits](https://www.conventionalcommits.org/) so auto-versioning works correctly
  - `feat:` -> minor bump, `fix:`/`perf:` -> patch bump, `feat!:` -> major bump
- [ ] If `self_update.rs` or artifact naming changed: `artifact_name()` updated to match

### Security

- [ ] No secrets, credentials, or API keys committed
- [ ] No new network calls without timeout handling
- [ ] External input (user args, file paths) is validated at the boundary

## Testing notes

<!-- Describe how you tested this. Which OS(es)? Manual steps? Edge cases covered? -->

## Related issues

<!-- Closes #... -->
