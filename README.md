# 🐸 rainfrog

a database management tui for postgres

![rainfrog demo](vhs/demo.gif)

> [!WARNING]
> rainfrog is currently in beta.

the goal for rainfrog is to provide a lightweight, terminal-based alternative to
pgadmin/dbeaver.

## features

- efficient navigation via vim-like keybindings and mouse controls
- query editor with keyword highlighting and session history
- quickly copy data, filter tables, and switch between schemas
- shortcuts to view table metadata and properties
- cross-platform (macOS, linux, windows, android via termux)

### why "rainfrog"?

> [frogs find refuge in elephant tracks](https://www.sciencedaily.com/releases/2019/06/190604131157.htm)

## disclaimer

this software is currently under active development; expect breaking changes,
and use at your own risk. it is not recommended to use this tool with write
access on a production database.

## installation

### cargo

after installing rust (recommended to do so via
[rustup](https://www.rust-lang.org/tools/install)):

```sh
cargo install rainfrog
```

### arch linux

arch linux users can install from the
[official repositories](https://archlinux.org/packages/extra/x86_64/rainfrog)
using [pacman](https://wiki.archlinux.org/title/pacman):

```sh
pacman -S rainfrog
```

### termux

if you are using [termux](https://termux.dev/), you'll need to install rust via
their package manager:

```sh
pkg install rust
```

and then make sure to install with termux features (and disable default
features):

```sh
cargo install rainfrog --features termux --no-default-features
```

### install script

there is a simple install script that assists in downloading and unpacking a
binary from the release page to `~/.local/bin/`, which you might want to add to
your `PATH` variable if it isn't already there. you'll need to select which
binary is appropriate for your system (if you're not sure, you can find out by
installing rust and running `rustc -vV` to see the "host" target), and the
script also needs [jq](https://github.com/jqlang/jq) and
[fzf](https://github.com/junegunn/fzf) installed to run.

```sh
curl -LSsf https://raw.githubusercontent.com/achristmascarl/rainfrog/main/install.sh | bash
```

### release page binaries

1. manually download and unpack the appropriate binary for your os from the
   latest [release](https://github.com/achristmascarl/rainfrog/releases) (if
   you're not sure which binary to pick, you can find out by installing rust and
   running `rustc -vV` to see the "host" target)
2. move the binary to a folder in your `PATH` environment variable

## usage

all arguments are optional; you will be prompted to provide any missing information.

```sh
rainfrog
```

### with individual options

if any options are not provided, you will be prompted to input them.
if you do not provide an input, that option will
default to what is in your environment variables.

```sh
rainfrog \
  --username <username> \
  --host <hostname> \
  --port <db_port> \
  --database <db_name>
```

### with connection url

the `connection_url` must include all the necessary options for connecting
to the database (ex. `postgres://username:password@localhost:5432/postgres`)

```sh
rainfrog --url $(connection_url)
```

### `docker run`

```sh
docker run --platform linux/amd64 -it --rm --name rainfrog \
  --add-host host.docker.internal:host-gateway \
  -e username="<username>" \
  -e password="<password>" \
  -e hostname="host.docker.internal" \
  -e db_port="<db_port>" \
  -e db_name="<db_name>" achristmascarl/rainfrog:latest
```

## keybindings

### general

| keybinding                   | description                   |
| ---------------------------- | ----------------------------- |
| `Ctrl+c`                     | quit program                  |
| `Alt+1`, `Ctrl+k`            | change focus to menu          |
| `Alt+2`, `Ctrl+j`            | change focus to query editor  |
| `Alt+3`, `Ctrl+h`            | change focus to query history |
| `Alt+4`, `Ctrl+g`            | change focus to results       |
| `Tab`                        | cycle focus forwards          |
| `Shift+Tab`                  | cycle focus backwards         |
| `q`, `Alt+q` in query editor | abort current query           |

### menu (list of schemas and tables)

| keybinding                   | description                       |
| ---------------------------- | --------------------------------- |
| `j`, `↓`                     | move selection down by 1          |
| `k`, `↑`                     | move selection up by 1            |
| `g`                          | jump to top of current list       |
| `G`                          | jump to bottom of current list    |
| `h`, `←`                     | focus on schemas (if more than 1) |
| `l`, `→`                     | focus on tables                   |
| `/`                          | filter tables                     |
| `Esc`                        | clear search                      |
| `Backspace`                  | focus on tables                   |
| `Enter` when searching       | focus on tables                   |
| `Enter` with selected schema | focus on tables                   |
| `Enter` with selected table  | preview table (100 rows)          |
| `R`                          | reload schemas and tables         |

### query editor

Keybindings may not behave exactly like Vim. The full list of active Vim keybindings in Rainfrog can be found at [vim.rs](./src/vim.rs).

| Keybinding        | Description                            |
| ----------------- | -------------------------------------- |
| `Alt+Enter`, `F5` | Execute query                          |
| `j`, `↓`          | Move cursor down 1 line                |
| `k`, `↑`          | Move cursor up 1 line                  |
| `h`, `←`          | Move cursor left 1 char                |
| `l`, `→`          | Move cursor right 1 char               |
| `w`               | Move cursor to next start of word      |
| `e`               | Move cursor to next end of word        |
| `b`               | Move cursor to previous start of word  |
| `0`               | Move cursor to beginning of line       |
| `$`               | Move cursor to end of line             |
| `gg`              | Jump to top of editor                  |
| `G`               | Jump to bottom of current list         |
| `Esc`             | Return to normal mode                  |
| `i`               | Enter insert (edit) mode               |
| `I`               | Enter insert mode at beginning of line |
| `A`               | Enter insert mode at end of line       |
| `o`               | Insert new line below and enter insert |
| `v`               | Enter visual (select) mode             |
| `V`               | Enter visual mode and select line      |
| `r`               | Begin replace operation                |
| `y`               | Begin yank (copy) operation            |
| `x`               | Begin cut operation                    |
| `p`               | Paste from clipboard                   |
| `u`               | Undo                                   |
| `Ctrl+r`          | Redo                                   |
| `Ctrl+e`          | Scroll down                            |
| `Ctrl+y`          | Scroll up                              |

### query history

| keybinding | description                   |
| ---------- | ----------------------------- |
| `j`, `↓`   | move selection down by 1      |
| `k`, `↑`   | move selection up by 1        |
| `g`        | jump to top of list           |
| `G`        | jump to bottom of list        |
| `y`        | copy selected query           |
| `I`        | edit selected query in editor |
| `D`        | delete all history            |

### results

| keybinding                | description                    |
| ------------------------- | ------------------------------ |
| `j`, `↓`                  | scroll down by 1 row           |
| `k`, `↑`                  | scroll up by 1 row             |
| `h`, `←`                  | scroll left by 1 cell          |
| `l`, `→`                  | scroll right by 1 cell         |
| `b`                       | scroll right by 1 cell         |
| `e`, `w`                  | scroll left by 1 column        |
| `{`, `PageUp`, `Ctrl+b`   | jump up one page               |
| `}`, `PageDown`, `Ctrl+f` | jump down one page             |
| `g`                       | jump to top of table           |
| `G`                       | jump to bottom of table        |
| `0`                       | jump to first column           |
| `$`                       | jump to last column            |
| `v`                       | select individual field        |
| `V`                       | select row                     |
| `Enter`                   | change selection mode inwards  |
| `Backspace`               | change selection mode outwards |
| `y`                       | copy selection                 |
| `Esc`                     | stop selecting                 |

## roadmap

<details>
  <summary><b>🏁 v0.1.0 – alpha</b></summary>
  
- [x] scrollable table
- [x] cancellable async querying (spawn tokio task)
- [x] menu list with tables and schemas (collapsable)
- [x] tui-textarea for query editor
- [x] basic tui-textarea vim keybindings
- [x] handle custom types / enums
- [x] display rows affected
- [x] confirm before delete/drop
- [x] table selection and yanking
- [x] multi-line pasting
- [x] editor os clipboard support
- [x] handle mouse events
- [x] keybindings hints at bottom
- [x] branch protection

</details>

<details>
  <summary><b>🏁 v0.2.0 – beta</b></summary>

- [x] vhs explainer gifs
- [x] upgrade ratatui and tui-textarea
- [x] shortcuts to view indexes, keys, etc.
- [x] performant syntax highlighting
- [x] session history
- [x] changelog, release script
- [x] handle explain / analyze output
- [x] show query duration
- [x] install script for bins

</details>

now that rainfrog is in beta, check out the
[issues tab](https://github.com/achristmascarl/rainfrog/issues) for planned
features

## known issues and limitations

- in addition to the experience being subpar if the terminal window is too
  small, if the terminal window is too large, rainfrog will crash due to the
  maximum area of ratatui buffers being `u16::MAX` (65,535). more details in
  <https://github.com/achristmascarl/rainfrog/issues/60>
- for query results with many columns, the height of the rendered `Table` widget
  may be limited due to the same limitation mentioned above. Could be fixed by
  <https://github.com/ratatui-org/ratatui/issues/1250>
- on mac, for VS Code and terminal (and perhaps other editors), a setting for
  "use option as meta key" needs to be turned on for Alt/Opt keybindings to
  work. (In VS Code, it's `"terminal.integrated.macOptionIsMeta": true`; in
  kitty, it's `macos_option_as_alt yes` in the config.)
- in visual mode, when selecting an entire line, the behavior is not the same as
  vim's, as it simply moves starts the selection at the head of the line, so
  moving up or down in lines will break the selection.
- mouse events are only used for changing focus and scrolling; the editor does
  not currently support mouse events, and menu items cannot be selected using
  the mouse

## acknowledgements

- [ratatui](https://github.com/ratatui-org/ratatui) (this project used ratatui's
  [component template](https://github.com/ratatui-org/templates/tree/983aa3cb3b8dd743200e8e2a1faa6e7c06aad85e/component/template)
  as a starting point)
- [tui-textarea](https://github.com/rhysd/tui-textarea) (used in the query
  editor)
- [gobang](https://github.com/TaKO8Ki/gobang) (a rust db tui i drew inspiration
  from)
- [ricky rainfrog](https://us.jellycat.com/ricky-rain-frog/)
- [rainfroggg](https://www.rainfrog.gg/) (my wife's tattoo studio)
