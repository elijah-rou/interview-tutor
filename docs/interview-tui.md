# Interview TUI

Run `./interview` in an interactive Linux terminal. Select a set and problem, open its detail, then press Enter again to load the selected language's planned source file into solve mode.

## Solve mode

The native editor supports a deliberately small Vim-style subset. Normal mode: `h j k l`, `w b`, `0 $`, `gg G`, `i a o O`, `x`, `dd`, `u`, and Ctrl-R. Insert mode: Unicode text, arrows, Backspace, Delete, Enter, and Esc. Command mode supports only `:w`, `:wq`, `:q`, and `:submit`. Unsupported commands report an error. Visual mode, search, macros, registers, plugins, and Vimscript are not supported. No Vim or Neovim process is launched.

Ctrl-S and F5 atomically save and run local tests without recording an attempt. A clean save still tests. F9 or `:submit` saves, tests, and records exactly one attempt after the runner terminates. Repeating submit records a new attempt. A failed save starts no runner. Ctrl-C cancels the active process group. Edits may continue while a test runs. STALE appears only when displayed run output belongs to an older revision; edits before the first run are not stale. `:wq` exits only when the saved and tested bytes still equal the current buffer, so edits made during that run remain open and dirty. Revisions never repeat across edits, undo, or redo, while dirty state compares saved bytes. At most the newest queued save/test is retained. Tab and Shift-Tab cycle Editor, Problem, Output, and Interview panes. Problem and Output accept Up/Down only while focused. Use `Space b` to leave solve mode or `Space q` to quit from Normal mode. With a dirty buffer, the first invocation shows a confirmation and the second identical invocation discards; edits or other actions clear confirmation. Esc never discards. `:q` rejects a dirty buffer, while `:wq` saves, tests, and exits only after the matching revision succeeds.

Cursor motion, deletion, insertion, and rendering use Unicode grapheme clusters, including combining marks and emoji ZWJ sequences. Normal mode addresses a grapheme; Insert mode addresses an insertion point.

On Linux, the source boundary anchors the canonical root and target parent with directory descriptors and uses `openat2` beneath/no-symlink/no-magic-link resolution. Only the catalog-planned regular file is accepted; symlinks, root escapes, FIFOs, invalid UTF-8, files over 1 MiB, and documents over 100,000 lines are rejected. Saves create an exclusive same-directory temporary through the parent descriptor, preserve mode through the file descriptor, sync data, rename within the anchored parent, and sync that parent. The editor keeps at most 32 undo snapshots.

Rust and Python receive deterministic keyword, string, and comment highlighting from the built-in bounded lexer. Other languages use plain text. A small lexer is used instead of Tree-sitter because only these three lexical classes are promised; this avoids grammar/runtime dependencies and is covered by exact unit and render-style tests.

At 100x30 and larger, Problem/Examples, Editor, offline Interview, and full-width Output/Test panes are visible. At 80x24, one selected pane is shown behind tabs. Below 60x20, only the resize panel is shown; it still reports the guarded `Space q` behavior and any active error. The Interview pane is intentionally offline until Stack 7.
