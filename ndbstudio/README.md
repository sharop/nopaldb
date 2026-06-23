# NDStudio 🌵

> **N**opal**D**B **Studio** - Interactive Terminal UI Explorer

A minimalist, keyboard-driven TUI for exploring NopalDB graph databases. Built for researchers, developers, and power users who prefer the terminal.

## Features ✨

### Core Capabilities
- 🔍 **Query Editor**: NQL syntax highlighting, multi-line editing, vim-style navigation
- 📊 **Results Viewer**: Scrollable tables, statistics, multiple view modes
- 📐 **Schema Browser**: Explore node types, edge types, properties, and database structure
- 🕐 **Query History**: Navigate and reuse previous queries (Ctrl+p/n)
- ⌨️ **Keyboard-First**: Complete vim-inspired keybinding system
- 🎨 **Minimalist Design**: Clean, distraction-free interface with gruvbox-inspired colors

### For Researchers
- Query plan visualization
- Data profiling and statistics
- Export to CSV/JSON/Arrow
- Reproducible query scripts

### For Developers
- Fast iteration on queries
- Schema introspection
- Error debugging
- Performance metrics

### For Users
- Intuitive navigation
- Clear visual feedback
- Helpful error messages
- Progressive disclosure of complexity

## Installation 🚀

### From Workspace (Recommended)

```bash
# From NopalDB root directory
cd nopaldb
cargo build --release

# NDStudio binary will be at:
# ./target/release/ndstudio
```

### Standalone Installation

```bash
cd ndstudio
cargo install --path .
```

## Quick Start 📖

```bash
# Open a database
ndstudio path/to/database.db

# Examples
ndstudio ./fraud_graph.db
ndstudio ~/data/synthetic_offshore.db
```

## Usage Guide ⌨️

### Modes

NDStudio operates in different modes, similar to vim:

- **Normal** - Navigation and commands
- **Insert** - Editing queries
- **Command** - Execute commands (`:`)
- **Visual** - Select results (future)
- **Schema** - Browse database structure

### Keybindings

#### Normal Mode
```
Movement:
  j/k         Scroll down/up in results
  h/l         Switch between panes (editor ↔ results)
  gg/G        Jump to top/bottom
  Ctrl-d/u    Page down/up
  
Pane Focus:
  1           Focus query editor
  2           Focus results pane
  Tab         Cycle through panes
  
Actions:
  i           Enter insert mode (edit query)
  :           Enter command mode
  s           Open schema browser
  v           Visual mode (future: select rows)
  <Enter>     Execute query
  
History:
  Ctrl-p      Previous query
  Ctrl-n      Next query
  
Exit:
  q           Quit (from Normal mode)
  Ctrl-c      Force quit (any mode)
```

#### Insert Mode
```
<Esc>           Return to Normal mode
Ctrl-Enter      Execute query and return to Normal
Arrow keys      Move cursor
Backspace       Delete character
Enter           New line
```

#### Command Mode
```
:q, :quit       Exit NDStudio
:schema         Open schema browser
:history        Show query history
:export csv     Export results to CSV
:export json    Export results to JSON
:export arrow   Export results to Arrow
:help           Show help message
```

#### Schema Mode
```
j/k             Scroll through schema
q, Esc          Return to Normal mode
```

## Example Workflow 🎯

### 1. Start NDStudio
```bash
ndstudio fraud_detection.db
```

### 2. Write a Query
Press `i` to enter insert mode, then type:

```nql
find n.name, pagerank(n) as score
from (n:Account)
where n.suspicious = true
order by score desc
limit 10
```

### 3. Execute
Press `Ctrl-Enter` (or `Esc` then `Enter`)

### 4. Explore Results
- Press `j/k` to scroll through results
- Press `2` to focus results pane
- Press `l` to move to results pane

### 5. Browse Schema
Press `s` to see database structure:
```
📦 Nodes (3 types)
  💼 Account (1.2M)
     • id: String (indexed)
     • balance: Float
     • suspicious: Bool
  🏢 Company (45K)
  👤 Person (890K)

🔗 Edges (2 types)
  💸 TRANSFER (3.8M)
  🏛️ OWNS (1.5M)
```

### 6. Export Results
```
:export csv
```

## Architecture 🏗️

```
ndstudio/
├── src/
│   ├── main.rs           # Entry point & event loop
│   ├── app.rs            # Application state machine
│   │
│   ├── ui/               # User interface components
│   │   ├── mod.rs        # Layout manager
│   │   ├── editor.rs     # Query editor widget
│   │   ├── results.rs    # Results table widget
│   │   ├── schema.rs     # Schema browser widget
│   │   └── history.rs    # Query history widget
│   │
│   ├── engine/           # Query execution
│   │   ├── mod.rs
│   │   └── executor.rs   # NopalDB integration
│   │
│   ├── commands/         # Command handlers
│   │   ├── mod.rs
│   │   └── handlers.rs   # :command implementations
│   │
│   └── config/           # Configuration
│       ├── mod.rs
│       └── keybindings.rs
│
├── Cargo.toml
└── README.md
```

## Design Philosophy 🎨

### Minimalist
- Show only essential information
- No visual clutter
- Clean borders and spacing
- Muted color palette

### Keyboard-First
- Every action accessible via keyboard
- Vim-inspired navigation
- No mouse required
- Fast for power users

### Progressive Disclosure
- Simple for basic tasks
- Advanced features available when needed
- Clear visual hierarchy
- Contextual help

### Performance
- Fast startup (<100ms)
- Responsive input (<16ms)
- Efficient rendering
- Handles large result sets

## Configuration 🔧

### Color Scheme
Edit `src/ui/mod.rs`:

```rust
pub const BG: Color = Color::Rgb(40, 40, 40);
pub const FG: Color = Color::Rgb(235, 219, 178);
pub const ACCENT: Color = Color::Rgb(184, 187, 38);
pub const ERROR: Color = Color::Rgb(251, 73, 52);
pub const SUCCESS: Color = Color::Rgb(142, 192, 124);
```

### Custom Keybindings
See `src/config/keybindings.rs` (coming soon)

## Development Status 📊

**Current Version**: 0.1.0-alpha

### Implemented ✅
- [x] Core TUI framework
- [x] Query editor with syntax highlighting
- [x] Results table view
- [x] Schema browser
- [x] Query history
- [x] Vim-style navigation
- [x] Command mode
- [x] NopalDB integration (in progress)

### In Progress 🔄
- [ ] Real NopalDB query execution
- [ ] Schema introspection from DB
- [ ] Export functionality
- [ ] Error display improvements

### Planned 🔮
- [ ] Autocomplete engine
- [ ] Query plan visualizer
- [ ] Statistics view
- [ ] Persistent history (SQLite)
- [ ] Saved queries
- [ ] Custom themes
- [ ] ASCII graph visualization

## Contributing 🤝

NDStudio is part of the NopalDB project. Contributions welcome!

### Development Setup

```bash
# Clone NopalDB
git clone https://github.com/sharop/nopaldb
cd nopaldb

# Build NDStudio
cargo build -p ndstudio

# Run
cargo run -p ndstudio -- test.db

# Run tests
cargo test -p ndstudio
```

### Code Style

```bash
# Format
cargo fmt

# Lint
cargo clippy -- -D warnings
```

## Troubleshooting 🔧

### Build Issues
```bash
# Clean and rebuild
cargo clean
cargo build --release
```

### Display Issues
```bash
# Check terminal support
echo $TERM

# Try different terminal
# Recommended: Alacritty, iTerm2, or modern terminal
```

### Performance Issues
```bash
# Build with optimizations
cargo build --release

# Profile with flamegraph
cargo install flamegraph
sudo cargo flamegraph -p ndstudio -- test.db
```

## FAQ ❓

**Q: Why TUI instead of GUI?**  
A: Faster for power users, works over SSH, lightweight, and keyboard-driven workflow is more efficient for database exploration.

**Q: Can I use it with large databases?**  
A: Yes! NDStudio uses streaming and pagination. Results are loaded incrementally.

**Q: How does it compare to graph database browser?**
A: NDStudio is terminal-based, keyboard-first, and designed for NopalDB's unique features like versioning and MVCC.

## Resources 📚

- [NopalDB Documentation](https://github.com/sharop/nopaldb)
- [NQL Language Guide](../../docs/nql.md)
- [ratatui Documentation](https://docs.rs/ratatui/)

## License 📄

AGPL-3.0 - See [LICENSE](../../LICENSE)

## Acknowledgments 🙏

- Built with [ratatui](https://github.com/ratatui-org/ratatui)
- Inspired by [lazygit](https://github.com/jesseduffield/lazygit) and [k9s](https://k9scli.io/)
- Part of the [NopalDB](https://github.com/sharop/nopaldb) ecosystem

---

**Built with**: Rust 🦀 • ratatui 🐀 • NopalDB 🌵

Maintained by the NopalDB community.
