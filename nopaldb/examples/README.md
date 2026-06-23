# NopalDB Game Examples 🎮

Three production-ready examples demonstrating NopalDB's graph database capabilities for game development.

## Examples

### 1. 🎯 RPG Quest System
`cargo run --example rpg_quest_system`

**Features:**
- Quest dependency chains
- Prerequisite checking
- Player progression tracking
- Dynamic quest unlocking

**Use Cases:**
- RPG quest systems
- Mission dependencies
- Story progression
- Achievement unlocking

---

### 2. 🌳 Skill Tree System
`cargo run --example skill_tree`

**Features:**
- Hierarchical skill dependencies
- Tier-based progression
- Multiple prerequisite paths
- Available skills calculation

**Use Cases:**
- Character progression
- Talent trees
- Tech trees (RTS games)
- Ability unlocking

---

### 3. 🤝 Character Relationship Network
`cargo run --example character_network`

**Features:**
- Social relationship graphs
- Faction systems
- Alliance/rivalry tracking
- Trade networks
- Friend-of-friend queries
- Reputation system

**Use Cases:**
- NPC relationship systems
- Social simulation
- Faction warfare
- Trade networks
- Reputation systems

---

## Quick Start
```bash
# Run all examples
cargo run --example rpg_quest_system
cargo run --example skill_tree
cargo run --example character_network

# Databases are created in ./examples/
ls examples/*.db
```

## NQL Query Examples

### Simple Node Query
```nql
find q.name, q.xp_reward
from (q:Quest)
where q.level_required <= 5
```

### Pattern Matching
```nql
find p.name, f.name
from (p:Player) -> [:COMPLETED] -> (q:Quest)
```

### 2-Hop Traversal
```nql
find friend.name, fof.name
from (c:Character) -> [:ALLIES_WITH] -> (friend) 
     -> [:ALLIES_WITH] -> (fof)
where c.name = "Arthur"
```

## Architecture

All examples use:
- ✅ ACID Transactions
- ✅ NQL Query Language
- ✅ Pattern Matching
- ✅ MVCC (optional)
- ✅ Persistent Storage

## Performance

- **Node creation:** ~100μs
- **Edge creation:** ~80μs
- **Simple query:** ~200μs
- **Pattern query:** ~500μs
- **2-hop query:** ~1ms

*Measured on M1 MacBook Pro*

## Next Steps

- Extend examples with more features
- Add visualization (Graphviz export)
- Add AI/ML integration
- Add real-time updates

## Learn More

- [NopalDB Documentation](../README.md)
- [NQL Query Language](../docs/NQL.md)
- [API Reference](../docs/API.md)
