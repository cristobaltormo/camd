# Contributing

Bug reports with the log (`journalctl -u camd` or `docker compose logs camd`) and the camera firmware version are the most useful contribution. Ideas are welcome as issues before code.

For a change:

1. fork, branch, and keep it focused;
2. run `ruff check . && pytest -q` and `node --test` in `server/`, and `pio run` in `firmware/` if you touched it;
3. add a test for what you changed and update the docs;
4. follow the conventions in [docs/development.md](docs/development.md): no comments, English identifiers, no emojis, translated interface text.

Commit messages are in the imperative: "Add X", "Fix Y when Z", with a body that says why.
