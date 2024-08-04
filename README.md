# 🐸 rainfrog
a database management tui for postgres

> [frogs find refuge in elephant tracks](https://www.sciencedaily.com/releases/2019/06/190604131157.htm) 

## disclaimer
this software is currently under active development; expect breaking changes, and use at your own risk.

## usage
`make dev url=$(connection_url)`

or

`cargo run -- -u $(connection_url)`

where `connection_url` includes the username:password for accessing the database (ex. `postgres://username:password@localhost:5432/postgres`)

## known issues
- for query results with many columns, the height of the rendered `Table` widget may be limited, as the maximum area of the underlying buffer is `u16::MAX` (65,535). Could be fixed by https://github.com/ratatui-org/ratatui/issues/1250

## roadmap
### v0.1.0
- [x] scrollable table 
- [x] cancellable async querying (spawn tokio task)
- [x] menu list with tables and schemas (collapsable)
- [x] tui-textarea for query editor
- [x] basic tui-textarea vim keybindings
- [x] handle custom types / enums
- [x] display rows affected
- [x] confirm before delete/drop
- [ ] table selection and yanking
- [ ] os clipboard support
- [ ] keybindings hints at bottom
- [ ] handle mouse events
- [ ] e2e tests
### v0.1.1
- [ ] handle explain / analyze output
- [ ] shortcut to view indexes, constraints, etc.
- [ ] loading animation
- [ ] table styling
- [ ] editor auto-complete
### backburner 
- [ ] syntax highlighting

## acknowledgements
- ratatui
- tui-textarea
- gobang
