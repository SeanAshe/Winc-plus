# Contributing to winc

Thank you for helping improve winc. This guide explains how to propose a change and what a pull
request needs before it can be merged.

## Before You Start

- Search the existing issues and pull requests first.
- For a larger change, such as a new feature, a new module, or a change to the rendering pipeline,
  open an issue to discuss it before writing code.
- Report security problems privately as described in [SECURITY.md](SECURITY.md), not in a public
  issue.
- Read and sign the [Contributor License Agreement](CLA.md). On your first pull request, a bot asks
  you to sign it by commenting on the pull request.

## Branches

- `dev` is the development branch and the repository's default branch. Fork the repository, create
  a branch from `dev`, and open your pull request against `dev`.
- `main` holds released code. Only maintainers merge `dev` into `main`, and a version change merged
  into `main` starts a release. Pull requests from contributors against `main` are retargeted to
  `dev` or closed.

## Setting Up

winc builds with Rust 1.99.1; `rust-toolchain.toml` selects the toolchain and installs rustfmt and
clippy.

```bash
cargo run --release                 # build and run
cargo xtask test                    # test the crates changed since HEAD
```

## Before Opening a Pull Request

CI checks formatting and runs clippy, the tests, and the documentation build on Windows for every
pull request that changes code. Run the same checks locally first; all of them
must pass without warnings:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask test --all
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

If your change can affect startup time, memory use, or scrolling, also run `cargo xtask bench` and
include the results in the pull request.

## Guidelines

- Keep each pull request focused on one change. Do not mix in unrelated refactoring or formatting.
- Run `cargo fmt` before committing. `rustfmt.toml` holds the settings, and CI rejects code that
  is not formatted.
- Add tests for fixed bugs and new behavior. Visual changes use the snapshot tests in `winc-app`.
- Follow the structure of the surrounding code: put logic in the crate that owns it (see the crate
  table in the [README](../README.md)), and keep platform-specific code behind the existing platform
  modules.
- Avoid new dependencies unless they are clearly needed. winc keeps its binary small and its startup
  fast, so every dependency is weighed against that.
- Interface strings live in the Fluent bundles of `winc-i18n`. Add new strings in both English
  (`en-US`) and Simplified Chinese (`zh-CN`).
- Write comments in a formal, neutral style, wrapped at 100 columns. Describe what the code does and
  why, not how it changed over time.

## Commit Messages

Write every commit subject, and every pull request title, in the form `type: subject`:

- `type` is one of `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`,
  `style`, or `test`, in lowercase, followed by a colon and one space. There is no scope in
  parentheses.
- `subject` describes the change in the imperative mood, in printable ASCII: English letters,
  digits, spaces, and ASCII punctuation such as `,` `.` `;` `(` `)` `"`. Non-ASCII characters,
  such as CJK text, full-width punctuation, or emoji, are rejected.
- The body, if any, is free-form. Explain why the change is needed, and wrap it at 72 columns.

Examples:

- `feat: add a minimap to the editor`
- `fix: keep the cursor column after undo`
- `perf: reuse glyph caches across frames, halving redraw time`
- `docs: explain the release workflow`

Pull requests are squash-merged into `dev`, so the pull request title becomes the commit message.
CI checks the title and the subject of every commit in the pull request, and the check fails until
they match. Some subjects that Git and GitHub write themselves are exempt:

- merge commits, for example after merging `dev` into your branch to resolve conflicts;
- reverts written by `git revert` or the GitHub Revert button (`Revert "feat: ..."`);
- autosquash commits (`fixup! ...`, `squash! ...`, `amend! ...`);
- for pull requests from `dev` into `main`, which are merged with a merge commit, titles that start
  with `Merge`.

To fix a commit subject, reword it with `git commit --amend` or `git rebase -i`, then force-push
your branch.

## Pull Request Review

A maintainer reviews every pull request. The checks "CI", "Commit messages", and "CLA" must pass
before a pull request is merged; a pull request into `main` must also pass "Source branch" and
"Release version". Maintainers may ask for changes, or close a pull request that does not fit the
project's direction.

## License

By contributing, you agree to the [Contributor License Agreement](CLA.md), and your contributions
are licensed under the [Apache License, Version 2.0](../LICENSE).
