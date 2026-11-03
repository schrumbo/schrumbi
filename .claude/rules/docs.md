# Docs

The in-app guide (Hilfe → Schrumbi-Anleitung) lives in `assets/docs/de/*.md` and `assets/docs/en/*.md`; pages are listed in `src/core/docs.rs`.

- Any change to user-facing behaviour updates the matching page, German and English, in the same commit.
- Never write keys or quoted labels by hand: `{key:quickOpen}` (shortcut id), `{key:GoToday}` (action in `bind_keys`), `{key:return}` (plain key); `{ui:Ablegen}` for a `tr()` label. Tests fail when one goes stale.
- Both languages keep the same sections, examples, heading levels and placeholders.
- Prose has no links; links belong in ````` ````example ````` blocks only.
- Little text, many examples. Check every claim against the running app.
