# 🐸 rainfrog
a database management tui for postgres

![rainfrog demo](demo.gif)

> [!WARNING]
> rainfrog is currently in alpha. 

the goal for rainfrog is to provide a lightweight, terminal-based alternative to pgadmin/dbeaver. 

### features
- efficient navigation via vim-like keybindings for query editor, data table, and menu
- quickly copy data, filter and preview tables, and switch between schemas
- cross-platform (macOS, linux, windows, android via termux)

### why "rainfrog"?
> [frogs find refuge in elephant tracks](https://www.sciencedaily.com/releases/2019/06/190604131157.htm) 

## disclaimer
this software is currently under active development; expect breaking changes, and use at your own risk. it is not recommended to use this tool with write access on a production database.

## installation
### cargo
after installing rust (recommended to do so via [rustup](https://www.rust-lang.org/tools/install)):
```sh
cargo install rainfrog --locked
```

### termux
if you are using [termux](https://termux.dev/), you'll need to install rust via their package manager:
```sh
pkg install rust
```
and then make sure to install with termux features (and disable default features):
```sh
cargo install rainfrog --locked --features termux --no-default-features
```

### binaries
1. download the appropriate binary for your os from the latest [release](https://github.com/achristmascarl/rainfrog/releases)
2. move the binary to a folder in your `PATH` environment variable

## usage
> [!NOTE]
> `connection_url` must include your credentials for accessing the database (ex. `postgres://username:password@localhost:5432/postgres`) 
```sh
rainfrog --url $(connection_url)
```

## keybindings
### general
| keybinding                  | description                            |
|-----------------------------|----------------------------------------|
| `Ctrl+c`                      | quit program                           |
| `Alt+1`, `Ctrl+m`               | change focus to menu                   |
| `Alt+2`, `Ctrl+n`               | change focus to query editor           |
| `Alt+3`, `Ctrl+b`               | change focus to results                |
| `q`                           | abort current query                    |

### menu (list of schemas and tables)
| keybinding                  | description                            |
|-----------------------------|----------------------------------------|
| `j`, `↓`                        | move selection down by 1               |
| `k`, `↑`                        | move selection up by 1                 |
| `g`                           | jump to top of current list            |
| `G`                           | jump to bottom of current list         |
| `h`, `←`                        | focus on schemas (if more than 1)      |
| `l`, `→`                        | focus on tables                        |
| `/`                           | filter tables                          |
| `Esc`                         | clear search                           |
| `Backspace`                   | focus on tables                        |
| `Enter` when searching        | focus on tables                        |
| `Enter` with selected schema  | focus on tables                        |
| `Enter` with selected table   | preview table (100 rows)               |
| `R`                           | reload schemas and tables              |

### query editor
keybindings may not behave exactly like vim. the full list of active vim keybindings in rainfrog can be found at [vim.rs](./src/vim.rs).
| keybinding             | description                            |
|------------------------|----------------------------------------|
| `Alt+Enter`, `F5`          | execute query                          |
| `j`, `↓`                   | move cursor down 1 line                |
| `k`, `↑`                   | move cursor up 1 line                  |
| `h`, `←`                   | move cursor left 1 char                |
| `l`, `→`                   | move cursor right 1 char               |
| `w`                      | move cursor to next start of word      |
| `e`                      | move cursor to next end of word        |
| `b`                      | move cursor to prev start of word      |
| `0`                      | move cursor to beginning of line       |
| `$`                      | move cursor to end of line             |
| `gg`                     | jump to top of editor                  |
| `G`                      | jump to bottom of current list         |
| `Esc`                    | return to normal mode                  |
| `i`                      | enter insert (edit) mode               |
| `I`                      | enter insert mode at beginning of line | 
| `A`                      | enter insert mode at end of line       |
| `o`                      | insert new line below and insert       |
| `v`                      | enter visual (select) mode             |
| `V`                      | enter visual mode and select line      |
| `r`                      | begin replace operation                |
| `y`                      | begin yank (copy) operation            |
| `x`                      | begin cut operation                    |
| `p`                      | paste from clipboard                   |
| `u`                      | undo                                   |
| `Ctrl+r`                 | redo                                   |
| `Ctrl+e`                 | scroll down                            |
| `Ctrl+y`                 | scroll up                              |

### results
| keybinding             | description                            |
|------------------------|----------------------------------------|
| `j`, `↓`                   | scroll down by 1 row                   |
| `k`, `↑`                   | scroll up by 1 row                     |
| `h`, `←`                   | scroll left by 1 cell                  |
| `l`, `→`                   | scroll right by 1 cell                 |
| `b`                      | scroll right by 1 cell                 |
| `e`, `w`                   | scroll left by 1 column                |
| `g`                      | jump to top of table                   |
| `G`                      | jump to bottom of table                |
| `0`                      | jump to first column                   |
| `$`                      | jump to last column                    |
| `v`                      | select individual field                |
| `V`                      | select row                             |
| `y`                      | copy selection                         |
| `Esc`                    | stop selecting                         |

## roadmap
<details>
  <summary><b>v0.1.0 (alpha)</b></summary>
  
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
  <summary><b>v0.2.0 (beta)</b></summary>

  - [x] vhs explainer gifs
  - [x] upgrade ratatui and tui-textarea 
  - [ ] change cursor insert-mode style
  - [ ] shortcuts to view indexes, keys, etc.
  - [ ] handle explain / analyze output
  - [ ] session history
  - [ ] unit / e2e tests
  - [ ] fix multi-line vim selections
  - [ ] non-vim editor keybindings
  - [ ] homebrew 
  - [ ] loading animation
</details>

<details>
  <summary><b>backburner</b></summary>

  - [ ] editor auto-complete
  - [ ] syntax highlighting
  - [ ] live graphs / metrics (a la pgadmin)
  - [ ] more packaging 
  - [ ] customization (keybindings, colors)
  - [ ] better vim multi-line selection emulation
  - [ ] handle more mouse events
  - [ ] support mysql, sqlite, other sqlx adaptors
  - [ ] vhs in cd
</details>

## known issues and limitations
- for query results with many columns, the height of the rendered `Table` widget may be limited, as the maximum area of the underlying buffer is `u16::MAX` (65,535). Could be fixed by https://github.com/ratatui-org/ratatui/issues/1250
- on mac, for VS Code and terminal (and perhaps other editors), a setting for "use option as meta key" needs to be turned on for Alt/Opt keybindings to work. (In VS Code, it's `"terminal.integrated.macOptionIsMeta": true`; in kitty, it's `macos_option_as_alt yes` in the config.)
- in visual mode, when selecting an entire line, the behavior is not the same as vim's, as it simply moves starts the selection at the head of the line, so moving up or down in lines will break the selection. 
- in visual mode, operations on backwards selections do not behave as expected. will be fixed after https://github.com/rhysd/tui-textarea/issues/80
- mouse events are only used for changing focus and scrolling; the editor does not currently support mouse events, and menu items cannot be selected using the mouse

## acknowledgements
- [ratatui](https://github.com/ratatui-org/ratatui) (this project used ratatui's [component template](https://github.com/ratatui-org/templates/tree/983aa3cb3b8dd743200e8e2a1faa6e7c06aad85e/component/template) as a starting point)
- [tui-textarea](https://github.com/rhysd/tui-textarea) (used in the query editor)
- [gobang](https://github.com/TaKO8Ki/gobang) (a rust db tui i drew inspiration from)
- [ricky rainfrog](https://us.jellycat.com/ricky-rain-frog/)
