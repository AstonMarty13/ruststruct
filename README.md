# ruststruct

[![CI](https://github.com/AstonMarty13/ruststruct/actions/workflows/ci.yml/badge.svg)](https://github.com/AstonMarty13/ruststruct/actions/workflows/ci.yml)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A tiny CLI that scaffolds a standard Rust project layout in one command — and
cleans up after itself if anything goes wrong.

```console
$ ruststruct myapp --git
Project `myapp` created successfully.

$ tree -a myapp
myapp
├── .git/
├── .gitignore
├── Cargo.toml          # written by `cargo init`
├── benches/
├── examples/
│   └── hello.rs
├── src/
│   ├── lib.rs
│   └── main.rs
└── tests/
    └── integration.rs

$ cd myapp && cargo run
Hello, myapp!
```

The generated project builds, runs, and passes its own tests immediately — a
lib/bin split with an example and an integration test already wired up.

## Install

```bash
cargo install --git https://github.com/AstonMarty13/ruststruct
```

Or from a clone:

```bash
git clone https://github.com/AstonMarty13/ruststruct
cd ruststruct
cargo install --path .
```

## Usage

```console
$ ruststruct --help
Scaffold a standard Rust project layout

Usage: ruststruct [OPTIONS] <PROJECT_DIR>

Arguments:
  <PROJECT_DIR>  Directory to create for the new project

Options:
  -n, --name <NAME>  Cargo package name [default: the project directory name]
      --git          Run `git init` once the project is in place
      --dry-run      Print the plan without writing anything
  -h, --help         Print help
  -V, --version      Print version
```

`--dry-run` prints exactly what would happen and touches nothing:

```console
$ ruststruct --dry-run --git myapp
[dry-run] project root : myapp
[dry-run] package name : myapp
[dry-run] directories  :
  myapp/benches/
  myapp/examples/
  myapp/src/
  myapp/tests/
[dry-run] files        :
  myapp/.gitignore
  myapp/examples/hello.rs
  myapp/src/lib.rs
  myapp/src/main.rs
  myapp/tests/integration.rs
[dry-run] would run    : cargo init --name myapp --vcs none
[dry-run] would run    : git init
```

## Configuration

Drop a `~/.ruststruct.json` to add your own directories and files to every new
project. Both keys are optional.

```json
{
  "dirs": ["scripts", "docs"],
  "files": {
    "rustfmt.toml": "edition = \"2024\"\n",
    ".github/workflows/ci.yml": "name: CI\n"
  }
}
```

- Entries are layered **on top of** the built-in defaults.
- A `files` key matching a built-in template (say `.gitignore`) replaces it.
- Parent directories are inferred, so `.github/workflows/ci.yml` just works.
- Unknown keys are rejected, so a typo like `"dir"` is reported instead of
  silently ignored.
- Paths that would escape the project root (`../`, absolute paths) are refused.

## Why this project exists

`ruststruct` is a deliberate port of [`gostruct`](https://github.com/AstonMarty13),
a small Go CLI of mine that does the same job for Go projects. Rewriting a tool I
already knew inside out turned the exercise into a direct comparison of the two
languages, rather than a tour of syntax.

Four things the type system made better, not just different:

**Rollback is a value, not a discipline.** The Go version tracks a `failed`
boolean and cleans up in a `defer` — correct only as long as every error path
remembers to set the flag. Here, a `RollbackGuard` deletes the project root when
it drops, and the happy path ends with `guard.disarm()`. It fires on `?` and on
panic, and there is no flag left to forget.

**Ordered maps make the dry run reproducible.** Go randomises map iteration, so
`gostruct --dry-run` lists directories in a different order on every run.
Swapping `HashMap` for `BTreeMap`/`BTreeSet` made the output stable — and made it
possible to assert on it in a test.

**Errors carry data instead of prose.** `fmt.Errorf("...: %w", err)` produces a
message; a `#[derive(Error)]` enum produces a value a caller can match on. The
rendered text is the same, the tests are not: they assert `Error::UnsafePath(_)`
rather than grepping a string.

**`&Path` beats `string`.** Path components are inspectable, so refusing anything
that is not `Component::Normal` is three lines — and it closes a hole the Go
version still has, where a config entry of `"../../.zshrc"` writes outside the
project.

The crate also holds itself to `#![forbid(unsafe_code)]`, `clippy::pedantic`, and
`missing_docs`, all enforced as errors in CI.

## Project layout

| Path | Role |
|---|---|
| `src/main.rs` | binary entry point — parse, load config, scaffold, report |
| `src/cli.rs` | `clap` derive definition and the mapping to scaffold options |
| `src/config.rs` | the optional `~/.ruststruct.json` |
| `src/scaffold.rs` | planning, applying, and the rollback guard |
| `src/error.rs` | one typed variant per failure mode |
| `tests/cli.rs` | end-to-end tests driving the real binary |

The logic lives in a library so that everything is reachable from tests; the
binary is a thin shell around it.

## Development

```bash
cargo test --all-features                                  # 36 tests
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

CI runs the lints, the MSRV check, and the test suite on Linux, macOS, and
Windows.

Two of the tests are worth pointing at: `generated_project_compiles_and_passes_its_own_tests`
runs `cargo test` inside a freshly scaffolded project, because a scaffolder that
emits code which does not build is worthless; and `dry_run_output_is_deterministic`
generates the same plan ten times and asserts the output never moves.

## License

MIT — see [LICENSE](LICENSE).
