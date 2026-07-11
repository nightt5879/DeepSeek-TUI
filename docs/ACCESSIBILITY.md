# Accessibility

DeepSeek-TUI runs in a terminal, so the platform's own accessibility
stack (screen readers, magnifiers, terminal-level themes) does most
of the work. The TUI provides a small set of toggles that reduce
visual motion and density for screen-reader and low-motion users.

## Quick reference

| Toggle | Default | Effect |
| --- | --- | --- |
| `NO_ANIMATIONS=1` env var | unset | At startup, forces `low_motion = true` and `fancy_animations = false`. Overrides whatever's saved in `settings.toml`. |
| `low_motion` setting | `false` | Freezes decorative and state animation without changing model text delivery. The footer water strip is controlled separately by `fancy_animations`. |
| `fancy_animations` setting | `true` | Enables expressive live-state chrome. Set to `false` to keep live-turn chrome still. |
| `ocean_treatment` setting | `ombre` | Chooses the background appearance: `ombre` paints the static water column; `flat` uses the theme surface. This is independent of motion settings. |
| `status_indicator` setting | `cw` | Static typographic header mark. Set to `whale` or `dots` for the legacy animations, or `off` to hide it. |
| `calm_mode` setting | `true` | Collapses tool-output details by default and trims status messages. Useful for screen readers that announce every redraw. |
| `show_thinking` setting | `true` | Set to `false` to hide model `reasoning_content` blocks entirely. |
| `show_tool_details` setting | `false` | Set to `true` to expand tool calls inline; details remain available on demand either way. |

## Standard env-var surface

Set these in your shell profile so they apply to every session:

```bash
# Force low-motion + no fancy animations.
export NO_ANIMATIONS=1

# Optional: respect the wider terminal-color convention.
export NO_COLOR=1            # honored by the underlying ratatui backend
```

`NO_ANIMATIONS` accepts any of `1`, `true`, `yes`, or `on`
(case-insensitive). Any other value (including `0`, `false`, empty,
or unset) leaves your saved settings alone.

The override is applied once at startup. Changing the env var
mid-session has no effect — settings are only re-read on the next
launch.

## Configuring via `/settings`

The same toggles are reachable from the command palette:

* `/settings set low_motion on`
* `/settings set fancy_animations off`
* `/settings set calm_mode on`
* `/settings set status_indicator off`

Settings written this way persist to `~/.codewhale/settings.toml` on new
installs, with legacy `~/.deepseek/settings.toml` and platform config-dir
settings kept as compatibility fallbacks.
The `NO_ANIMATIONS` env var still wins at startup if it's set, so
unsetting the env var is the way to honor your saved choice.

Tilix and Terminator sessions automatically start in low-motion mode because
those VTE-based terminals have reported visible redraw flicker during active
turns. You can still override the saved settings after launch if your terminal
version renders cleanly.

## Notes for screen-reader users

* `low_motion` slows the idle redraw loop to ~120ms per frame and freezes state
  markers without synthesizing or throttling model text. Combined with
  `calm_mode`, the redraw rate stays low enough that VoiceOver /
  Orca announcements track linearly with model output instead of
  re-reading the whole screen on each tick.
* The transcript is pure text — no images or canvas rendering — so
  any terminal that integrates with the platform's accessibility
  service (e.g. macOS Terminal.app, iTerm2, Ghostty, Windows
  Terminal) will pass the rendered content straight through.
* If you find a UI surface that still produces motion when
  `low_motion = true`, please file an issue against
  [`PRIOR: Screen-reader / accessibility flag`](https://github.com/Hmbown/CodeWhale/issues/450)
  with a screenshot or terminal recording.

## Related issues / history

* [#450](https://github.com/Hmbown/CodeWhale/issues/450) —
  documenting the existing flag, adding the `NO_ANIMATIONS`
  startup overlay, and writing this page.
* [#449](https://github.com/Hmbown/CodeWhale/issues/449) —
  footer statusline now uses the active theme's contrast pair
  instead of a bespoke palette.
