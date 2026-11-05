## What and why

## Checklist

- [ ] `ruff check . && ruff format --check . && pytest -q` pass in `server/`
- [ ] `node --test webtests/` passes if the interface changed
- [ ] `pio run` passes if the firmware changed
- [ ] New behaviour has a test
- [ ] Documentation and `CHANGELOG.md` are updated
- [ ] Interface text goes through `t()` and has a Spanish translation
- [ ] No comments in code, no emojis, no secrets
