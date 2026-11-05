# Contributing

Thanks for wanting to help. satchel.email is maintained by one person, so the process is kept small.

## Issues

Bug reports and feature ideas are welcome as GitHub issues. For a bug, include the kind of account (Gmail, Microsoft 365, IMAP, JMAP), what you did, what you expected and what happened. Don't paste real mail or addresses you don't want public.

**Security problems:** don't open a public issue. Use *Report a vulnerability* on the repository's Security tab.

## Pull requests

1. **Open an issue first** for anything bigger than a small fix, so we can agree on the approach before you spend time on it. A pull request that arrives without one may be closed.
2. **Sign the CLA.** On your first pull request the CLA bot asks you to agree to the [Contributor License Agreement](CLA.md) with a comment. You keep the copyright in your work; the CLA lets the project use it under any license, which keeps a hosted version and future licensing open.
3. **Follow the ground rules** in [CLAUDE.md](CLAUDE.md) (written for coding agents, but they apply to people too). Above all: agents may only read and propose, never act directly; everything goes through `satchel_api::Command`; no new dependency without a good reason; and a significant design decision gets a record in [docs/decisions/](docs/decisions/README.md).
4. **Run the tests** before you push:

   ```sh
   cargo test --workspace
   (cd ui && npx tsc --noEmit)
   ```

## Licenses of contributions

Your contribution is released under the license of the folder it's in: AGPL-3.0 for the app, and MIT OR Apache-2.0 for the `mime`, `ical` and `api` crates. See the License section of the [README](README.md).
