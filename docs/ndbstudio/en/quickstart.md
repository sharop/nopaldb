# NDBStudio Quickstart (EN)

> For the local web workbench, see [NDBStudio Web Quickstart](../web_quickstart.md).

## Run

From repository root:

```bash
cargo run -p ndbstudio -- ./path/to/your.db
```

Example:

```bash
cargo run -p ndbstudio -- ../synthetic_offshore/data/synthetic_offshore.db
```

Disable loading screen:

```bash
NDBSTUDIO_NO_LOADING=1 cargo run -p ndbstudio -- ./path/to/your.db
```

## Basic flow

1. Write a query in `Editor`.
2. Execute with `Enter` (NORMAL mode) or `Ctrl+Enter` (INSERT mode).
3. Review `Results`.
4. Switch result modes with `t` or `:results <mode>`.

## First useful query

```sql
find c.name, c.house
from (c:Character)
limit 20
```

## Exit

- `:q` or `:quit`
