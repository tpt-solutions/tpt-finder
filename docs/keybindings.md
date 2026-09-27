# Keybindings

## Global

| Shortcut | Action |
| --- | --- |
| `Ctrl+Space` (configurable) | Summon / dismiss the quick-search popup from anywhere |

Rebind in *Settings → Global hotkey* using Tauri shortcut syntax, e.g. `Alt+Space`, `CommandOrControl+P`, `Ctrl+Shift+F`. The hotkey is validated before it is saved; if registration fails (another app owns it), the save reports the error and keeps the previous binding.

## Popup overlay

| Key | Action |
| --- | --- |
| type | instant search-as-you-type |
| `↑` / `↓` | move selection |
| `Home` / `End` | first / last result |
| `Enter` | open selected result (popup dismisses) |
| `Ctrl+Enter` | reveal selected result in file manager |
| `Esc` | dismiss popup |
| blur | popup auto-dismisses when it loses focus |

## Main window

| Key | Action |
| --- | --- |
| `/` | focus the search box |
| `↑` / `↓`, `Home` / `End` | move selection in the results list |
| `Enter` (results focused) | open selected result |
| `Esc` (in search box) | clear the query |
| click column headers | toggle sort direction; sort choice persists |

Result rows expose Open / Reveal / Copy path / Copy file / Delete actions on hover or selection; the preview pane mirrors them for the current selection.
