# 🐸 rainfrog
a database management tui for postgres

> [frogs find refuge in elephant tracks](https://www.sciencedaily.com/releases/2019/06/190604131157.htm) 

## usage
`make dev url=$(connection_url)`

or

`cargo run -- -u $(connection_url)`

where `connection_url` includes the username:password for accessing the database (ex. `postgres://username:password@localhost:5432/postgres`)

## known issues
- for query results with many columns, the height of the rendered `Table` widget may be limited, as the maximum area of the underlying buffer is `u16::MAX` (65,535)

## TODO
- [x] scrollable table 
- [x] cancellable async querying (spawn tokio task)
- [x] menu list with tables and schemas (collapsable)
- [x] tui-textarea for query editor
- [x] basic tui-textarea vim keybindings
- [x] handle custom types / enums
- [ ] display rows affected
- [ ] table footer
- [ ] table selection and yanking
- [ ] keybindings hints at bottom
- [ ] handle mouse events
- [ ] handle explain / analyze output
- [ ] confirm before delete (wrap in transactions)
- [ ] editor syntax highlighting
- [ ] view indexes, constraints, etc.
- [ ] loading animation
- [ ] table styling
- [ ] perf (limit rendering)
- [ ] improved tui-textarea vim keybindings
