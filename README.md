# 🐸 rainfrog
a database management tui for postgres

> [frogs find refuge in elephant tracks](https://www.sciencedaily.com/releases/2019/06/190604131157.htm) 

## usage
`make dev url=$(url)`

or

`cargo run -- -u $(url)`

## TODO
- [x] scrollable table 
- [x] cancellable async querying (spawn tokio task)
- [x] menu list with tables and schemas (collapsable)
- [x] tui-textarea for query editor
- [ ] loading state when querying
- [ ] keybindings hints at bottom
- [ ] table selection and yanking
- [ ] handle mouse events
- [ ] tui-textarea vim keybindings
- [ ] handle explain / analyze output
- [ ] confirm before delete
- [ ] editor syntax highlighting
- [ ] view indexes, constraints, etc.
- [ ] table styling
- [ ] perf (limit rendering)
