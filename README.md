# 🐸 rainfrog
a database management tui for postgres

> [frogs find refuge in elephant tracks](https://www.sciencedaily.com/releases/2019/06/190604131157.htm) 

## usage
`make dev`

or

`cargo run -- -u $(url)`

## TODO
- [x] scrollable table 
- [x] cancellable async querying (spawn tokio task)
- [ ] menu list with tables and schemas (collapsable)
- [ ] loading state when querying
- [ ] tui-textarea for query editor
- [ ] keybindings hints at bottom
- [ ] table selection and yanking
- [ ] handle mouse events
- [ ] handle explain / analyze output
- [ ] confirm before delete
- [ ] table styling
- [ ] perf (limit rendering)
