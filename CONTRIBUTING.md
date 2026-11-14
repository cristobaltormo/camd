# Contributing

Bug reports with the log (`journalctl -u camd` or `docker compose logs camd`), the output of `camd --version` and the camera firmware version are the most useful contribution. Ideas are welcome as issues before code.

For a change:

1. fork, branch, and keep it focused;
2. run `make test` (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `node --test` in `tests/web`), `make ui` if you touched the interface, and `pio run` in `firmware/` if you touched it;
3. add a test for what you changed and update the docs and `CHANGELOG.md`;
4. follow the conventions in [docs/development.md](docs/development.md): no comments, English identifiers, no emojis, translated interface text, no work per frame that can be done per event.

Commit messages are in the imperative: "Add X", "Fix Y when Z", with a body that says why.
