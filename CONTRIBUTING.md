# CONTRIBUTING

## AI-generated contributions
contributions in any form (code, documentation, issues, PRs, comments, etc.) that use AI/LLM tools should adhere to the following guidelines:
1. disclose that AI/LLMs were used and how they were used in the contribution
2. the contributor is responsible for verifying the quality and correctness of the contribution
3. avoid large AI-generated contributions; each pull request should be scoped with human reviewers in mind
4. the contributor should be prepared to discuss the rationale behind the contribution and any decisions made in the code
5. the contributor may be asked to make specific changes to the contribution to ensure it meets the standards of this project

failure to follow these guidelines will result in the contribution being rejected (issue/PR closed) or reverted.
if AI usage is not disclosed and maintainers are unable to determine whether a contribution was AI-generated (based on
the contributor's ability to answer questions about the contribution, their contribution history, or any other heuristic
used by reviewers), maintainers are at liberty to reject the contribution without further discussion.

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
cargo fmt --all --check
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
