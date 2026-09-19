# Dispatch capture shortcut audit

Audit scope: desktop timeline/message rows, channel and thread composers,
composer extensions/autocomplete, and the app shortcut registry on base
5ba609485. Capture uses existing dialogs and the Dispatch feature gate.

| Existing shortcut | Owner and behavior | Capture collision check |
| --- | --- | --- |
| Enter | `useRichTextEditor`: send; code fence handling; autocomplete accepts a result first | Different key; `/track` uses the same send pipeline |
| Shift+Enter | `useRichTextEditor`: hard break, list/quote continuation | Different key |
| ArrowUp in empty composer | `useRichTextEditor`: edit last own message | Different key; capture does not affect edit mode |
| ArrowUp / ArrowDown | `useMentions`, `useChannelLinks`, `useEmojiAutocomplete`: navigate suggestions | Different key |
| Tab / Enter / Escape | Composer autocomplete: accept/dismiss; Tab settles link editor; Escape cancels edit | Different keys; T ignores editable targets |
| Cmd+Shift+M / Ctrl+Shift+M | `useAlwaysAddressShortcut`: address agent | Different key |
| Cmd+B / I / E / U / Shift+S | Tiptap: bold, italic, inline code, underline, strike | Different keys; T ignores editor input |
| Cmd+K / Ctrl+K | Composer link editor when it owns selection, otherwise app search | Different key |
| Cmd+Shift+V / Ctrl+Shift+V | Plain-text paste in `useRichTextEditor` | Different key |
| Ctrl+A / E | macOS visual-line start/end | Different keys |
| Ctrl+B / F / K | `macEmacsTextShortcuts`: macOS cursor movement and kill-line | Different keys; T requires no modifiers and Cmd+Shift+T rejects Ctrl |
| Cmd+Z / Shift+Z / Ctrl+Y | Tiptap undo/redo | Different keys |
| Cmd+Alt+0…6 / C; Cmd+Shift+7 / 8 / B; Tab / Shift+Tab in lists; Mod+Delete / Backspace; Mod+Enter | StarterKit block/list commands | Different keys and editor-only ownership |
| Escape / Shift+Escape | App read state; dialogs and focus-thread drawer close | Different keys; capture ignores open dialogs/menus |
| Ctrl+Space / Ctrl+Shift+Space | Huddle push-to-talk / toggle | Different key |
| Cmd+F / K / Shift+K / Shift+O / Shift+N / Shift+A / S / comma / brackets; zoom keys | `keyboard-shortcuts.ts` and app handlers: navigation, settings, search, zoom | No registered T binding |
| ArrowDown in empty final list item | `useRichTextEditor`: exits the list | Different key |
| Plain T (new) | Focused message row or its action buttons opens Track dialog | Ignores editable descendants, modifiers, composition, repeats, and already-handled keys |
| Cmd+Shift+T (new) | Channel capture opens newest loaded own delivered message | No Buzz timeline/composer binding; ignored during composition, dialogs and menus. Browser hosts conventionally reserve it for reopen-tab; native Buzz is the target |

A focused message row is a keyboard stop so the plain T command has an explicit
owner. The ordinary letter `t` remains text in the composer. Both shortcuts only
open review; neither publishes an issue on a keypress.

The pin toggle persists via existing workflow CRUD with expected revision and
community/signer fencing. Its bridge tests assert the definition, not relay
execution; live reaction execution belongs to L3.
