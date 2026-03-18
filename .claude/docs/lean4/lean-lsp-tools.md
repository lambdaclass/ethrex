# Lean LSP MCP Tools Reference

Tools provided by lean-lsp-mcp (Numina fork). All tool names prefixed with `mcp__plugin_lean4-toolkit_lean-lsp__`.

## Local Tools (unlimited, instant)

### lean_goal
Check proof state at a position. Use BEFORE writing any tactic.
- `file_path`: absolute path
- `line`: 1-indexed
- `character`: 1-indexed column
- Returns: goals_before, goals_after (structured)
- Empty goals_after = proof complete

### lean_diagnostic_messages
Get all errors/warnings for a file. Use AFTER every edit.
- `filePath`: relative to project root
- Returns: array of {severity, range, message}

### lean_hover_info
Get type/documentation for a symbol.
- `filePath`, `line`, `character`
- Returns: type signature, documentation

### lean_completions
Get available completions at a position.
- `filePath`, `line`, `character`
- Returns: array of completion items

### lean_file_outline
Get structural outline of declarations.
- `filePath`
- Returns: array of {name, kind, range}

## Rate-Limited Tools (3 req/30s each)

### lean_loogle
Type-based search on loogle.lean-lang.org.
- Query with type patterns: `(?a -> ?b) -> List ?a -> List ?b`

### lean_leansearch
Natural language semantic search on leansearch.net.
- Query with descriptions: "continuous function on compact space"

## Best Practices

1. **Always local first**: lean_goal, lean_diagnostic_messages, lean_hover_info
2. **Rate-limited sparingly**: lean_loogle, lean_leansearch (3 req/30s)
3. **lean_goal before every tactic**: know what you're proving
4. **lean_diagnostic_messages after every edit**: catch errors instantly (<1s vs 10-30s build)
5. **lean_hover_info to confirm API**: check function signatures before using them
