# Third-party notices

satchel.email bundles no third-party fonts, images or data files: the UI's only asset is its own icon (`ui/public/favicon.svg`).

What ships in a release built by `scripts/release.sh` is the project's code plus its Rust crates and npm packages (listed in `Cargo.lock` and `ui/package-lock.json`). Each keeps its own license, found in its package. [docs/decisions/0003-dependencies.md](docs/decisions/0003-dependencies.md) records why each one is there.

`scripts/release.sh` collects their license texts with `scripts/third-party-licenses.py` into `THIRD_PARTY_LICENSES.txt`, which ships in the app's resources (with `LICENSE`) and beside the `satchel` binary. A crate that ships no license file gets the standard text of the license it declares; the script names any it can't cover, to check by hand before the release.
