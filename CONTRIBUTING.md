# CONTRIBUTING

the codebase is not currently in a great place for delegating issues to external
contributors, as there aren't that many tests and certain sections are overdue
for a refactor. that being said, bug reports and feature requests are always
welcome, and if you see an issue you'd like to work on, you are welcome to make
a pull request fully addressing it if it's a small enough issue, or you can reach
out via email to [carl@rainfrog.dev](mailto:carl@rainfrog.dev) or open a draft pull
request outlining a prototype of how you'd approach it. i'll do my best to work
with you to get it merged, but i can't make any guarantees given the current
state of the project.

## Bug reports

open an issue using the appropriate issue template.

## Feature requests

open an issue using the appropriate issue template.

## Pull requests

if you are tackling a complicated issue, please reach out via email
to [carl@rainfrog.dev](mailto:carl@rainfrog.dev) or open a draft pull request
to start a discussion before making too much progress.

### Formatting

make sure to check the format before opening a PR by running:

```sh
cargo fmt --workspace --check
```

### Tests

make sure all tests pass before opening a PR by running:

```sh
cargo test --workspace --all-features
```

### Clippy

run clippy and fix any issues before opening a PR by running:

```sh
cargo clippy --all-targets --all-features --workspace -- -D warnings
```

### CI

in addition to the tests and formatting, the CI workflow will run
tests on multiple targets when a pull request is opened. it's okay
if you aren't able to test for multiple platforms locally and to
catch those issues in CI, but they will need to be fixed before merging.
