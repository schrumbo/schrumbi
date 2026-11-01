---
paths:
  - "src/ui/**"
---

# UI

Every interactive element needs all of these, or a stated reason:

- Hover and press: `.soft(color, radius)` from `ui/motion.rs` on `Stateful<Div>`, before adding children. kit buttons have no soft hover.
- Keyboard: `.key_nav()` from `ui/widgets.rs`, and the action reachable without a mouse.
- Pointer: `.cursor_pointer()`. Icon-only controls get `.tooltip(tip(tr("...")))`.
- States: disabled, loading, empty, error and selected are designed, not defaulted, in light and dark.

Values come from `ui/theme.rs`: `space`, `iconsize`, `control`, `radius`, `font`, colors from `Theme`. Motion comes from `ui/motion.rs`: `FAST`, `MEDIUM`, `SLOW`, `follow`, `reveal`, `chevron`, `flip`, `spinner`, `loading`, and `timed` for `with_animation` so reduced motion is respected. New values go into a token, not inline.

Feel: the next frame reacts. No blocking work on the UI thread. Motion is interruptible, under 300 ms, and absent on frequent keyboard actions. Everything undoable goes through `Library::commit` / `record_edit`.

The standard is Figma. Before designing or changing an interaction, read `~/.claude/skills/app-polish/references/feel.md`; for workflows also `flows.md`, for input `inputs.md`, in the same folder.

## GPUI traps

- Elements cannot be scaled.
- `request_animation_frame` only in render; in a handler it crashes.
- Updating an entity from inside its own update panics. In subscriptions on input fields use App-level `cx.subscribe`, and move focus changes to the next frame.
- `group_drag_over` needs its own hitbox: add `.hover(|s| s)`.
- Inputs let Enter propagate, so an action and `PressEnter` can both fire.
- An icon name with no file in `assets/icons` renders as nothing. Add missing ones from lucide-static 0.460.0.
- Default line height is about 1.6; the root sets `line_height(relative(1.22))`.
