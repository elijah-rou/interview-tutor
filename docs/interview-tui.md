# Interview TUI

Run `./interview` in an interactive Linux terminal. Select a set and problem, open its detail, then press Enter again to load the selected language's planned source file into solve mode.

## Solve mode

The native editor supports a deliberately small Vim-style subset. Normal mode: `h j k l`, `w b`, `0 $`, `gg G`, `i a o O`, `x`, `dd`, `u`, and Ctrl-R. Insert mode: Unicode text, arrows, Backspace, Delete, Enter, and Esc. Command mode supports only `:w`, `:wq`, `:q`, and `:submit`. Unsupported commands report an error. Visual mode, search, macros, registers, plugins, and Vimscript are not supported. No Vim or Neovim process is launched.

Ctrl-S and F5 atomically save and run local tests without recording an attempt. A clean save still tests. F9 or `:submit` saves, tests, and records exactly one attempt after the runner terminates. Repeating submit records a new attempt. A failed save starts no runner. Ctrl-C cancels the active process group. Edits may continue while a test runs; an older generation is marked stale and cannot describe the current buffer. At most the newest queued save/test is retained. Tab and Shift-Tab cycle Editor, Problem, Output, and Interview panes. Use `Space q` to quit from Normal mode; dirty buffers are guarded.

The source boundary accepts only the catalog-planned regular file under the canonical project root. Symlinks, root escapes, invalid UTF-8, files over 1 MiB, and documents over 100,000 lines are rejected. Saves use a same-directory temporary file, preserve Unix mode, sync data, rename, and sync the parent. The editor keeps at most 32 undo snapshots.

Rust and Python receive deterministic keyword, string, and comment highlighting from the built-in bounded lexer. Other languages use plain text. A small lexer is used instead of Tree-sitter because only these three lexical classes are promised; this avoids grammar/runtime dependencies and is covered by exact unit and render-style tests.

At 100x30 and larger, Problem/Examples, Editor, offline Interview, and full-width Output/Test panes are visible. At 80x24, one selected pane is shown behind tabs. Below 60x20, only the resize panel is shown. The Interview pane is intentionally offline until Stack 7.
