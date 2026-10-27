# NopalDB Examples 💡

Real-world use cases and query patterns.

---

## 🌐 Social Network Analysis

```python
import nopaldb

graph = nopaldb.Graph.open("social.db")  # a new, empty directory
tx = graph.begin_transaction()

# Users
users = {}
for name in ["Alice", "Bob", "Charlie", "Diana"]:
    uid = tx.add_node("User", {
        "name": name,
        "joined": "2024-01-01"
    })
    users[name] = uid

# Relationships (Follows)
tx.add_edge(users["Alice"], users["Bob"], "FOLLOWS", {"since": 2020})
tx.add_edge(users["Bob"], users["Charlie"], "FOLLOWS", {"since": 2021})
tx.add_edge(users["Alice"], users["Diana"], "FOLLOWS", {"since": 2019})
tx.add_edge(users["Charlie"], users["Alice"], "FOLLOWS", {"since": 2022})
tx.add_edge(users["Diana"], users["Bob"], "FOLLOWS", {"since": 2023})

tx.commit()

# Analysis: Most followed users (f follows u)
result = graph.execute_nql("""
    find u.name, count(*) as followers
    from (f:User)-[:FOLLOWS]->(u:User)
    group by u.name
    order by followers desc, u.name
""")

for row in result:
    print(f"{row.get('u.name')}: {row.get('followers')} followers")
# Bob: 2 followers
# Alice: 1 followers
# Charlie: 1 followers
# Diana: 1 followers
```

The anonymous form `from ()-[:FOLLOWS]->(u:User)` is the same query. Before
0.9.4 it could bind `u` to the *source* and count wrongly (#293); naming the
source, as above, gives the right counts on every version.

---

## 🏢 Fraud Detection (Transaction Cycles)

Detecting money laundering often involves finding cycles where money moves between accounts and returns to the origin.

```python
tx = graph.begin_transaction()
acc = {n: tx.add_node("Account", {"name": n, "risk_score": r})
       for n, r in [("A1", 0.9), ("A2", 0.2), ("A3", 0.4), ("A4", 0.7)]}
for s, t in [("A1", "A2"), ("A2", "A3"), ("A3", "A1"), ("A3", "A4")]:
    tx.add_edge(acc[s], acc[t], "TRANSFER", {"amount": 5000})
tx.commit()

result = graph.execute_nql("""
    // 3-hop cycles: a -> b -> c -> a
    find a.name, b.name, c.name
    from (a:Account)-[:TRANSFER]->(b:Account)-[:TRANSFER]->(c:Account)-[:TRANSFER]->(a:Account)
    where a.risk_score > 0.5
""")
print(list(result))
# [{'a.name': 'A1', 'b.name': 'A2', 'c.name': 'A3'}]
```

A variable that appears twice in a pattern is the **same node**: the second
`(a:Account)` closes the cycle on the first one (since 0.9.4, #293; before it
was re-bound and `A4`, which is not in any cycle, also came back). Each cycle
is found once per starting node that passes the `where`; here only `A1` has
`risk_score > 0.5`.

---

## 🛍️ Recommendation System (Collaborative Filtering)

Finding products purchased by users who bought the same items. NQL returns
the paths; the counting is done in Python, because aggregations over
multi-hop patterns are not supported yet (`count(*)` with `group by` on this
pattern is an error that says so).

```python
from collections import Counter

tx = graph.begin_transaction()
buyers = {n: tx.add_node("User", {"name": n}) for n in ["ana", "beto", "carla"]}
items = {n: tx.add_node("Product", {"name": n}) for n in ["seeds", "shovel", "gloves", "pot"]}
for who, what in [("ana", "seeds"), ("ana", "shovel"), ("beto", "seeds"), ("beto", "shovel"),
                  ("beto", "gloves"), ("carla", "seeds"), ("carla", "gloves"), ("carla", "pot")]:
    tx.add_edge(buyers[who], items[what], "BOUGHT")
tx.commit()

rows = graph.execute_nql("""
    find p2.name
    from (u:User {name: "ana"})-[:BOUGHT]->(p1:Product)<-[:BOUGHT]-(u2:User)-[:BOUGHT]->(p2:Product)
    where u2.name != "ana" and p2.name != p1.name
""")
owned = {"seeds", "shovel"}   # what ana already bought
common = Counter(r["p2.name"] for r in rows if r["p2.name"] not in owned)
print(common.most_common(5))
# [('gloves', 3), ('pot', 1)]
```

---

## 🛡️ Access Control (RBAC)

Checking if a user has permission via role inheritance.

```nql
// Path: User -> Role -> Permission
find p.name
from (u:User {username: "alice"})-[:HAS_ROLE]->(r:Role)-[:HAS_PERM]->(p:Permission)
```

---

## 📊 Data Science Pipeline (Arrow Export)

Export graph data efficiently for machine learning models.

```python
import pyarrow as pa
import pandas as pd
from sklearn.ensemble import RandomForestClassifier

# 1. Export data: an Arrow IPC stream (one copy, no per-row parsing).
#    With a label, each property of those nodes is a column.
nodes_bytes = graph.to_arrow(label="Customer")
df = pa.ipc.open_stream(nodes_bytes).read_all().to_pandas()

# 2. Feature Engineering
features = df[['age', 'total_spend', 'visits']]
target = df['churned']

# 3. Train Model
clf = RandomForestClassifier()
clf.fit(features, target)
print("Model trained!")
```
