# NopalDB Examples 💡

Real-world use cases and query patterns.

---

## 🌐 Social Network Analysis

```python
import nopaldb

graph = nopaldb.Graph.open("social.db")
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

tx.commit()

# Analysis: Most followed users
result = graph.execute_nql("""
    find u.name, count(*) as followers
    from ()-[:FOLLOWS]->(u:User)
    group by u.name
    order by followers desc
""")

for row in result:
    print(f"{row.get('u.name')}: {row.get('followers')} followers")
```

---

## 🏢 Fraud Detection (Transaction Cycles)

Detecting money laundering often involves finding cycles where money moves between accounts and returns to the origin.

```nql
-- Find 3-hop cycles: A -> B -> C -> A
find a.id, b.id, c.id
from (a:Account)-[:TRANSFER]->(b:Account)-[:TRANSFER]->(c:Account)-[:TRANSFER]->(a:Account)
where a.risk_score > 0.5
```

---

## 🛍️ Recommendation System (Collaborative Filtering)

Finding products purchased by users who bought similar items.

```nql
find p2.name, count(*) as common_purchases
from (u:User)-[:BOUGHT]->(p1:Product)<-[:BOUGHT]-(u2:User)-[:BOUGHT]->(p2:Product)
where u.id = "user_123" 
  and p2.name != p1.name
group by p2.name
order by common_purchases desc
limit 5
```

---

## 🛡️ Access Control (RBAC)

Checking if a user has permission via role inheritance.

```nql
-- Path: User -> Role -> Permission
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

# 1. Export Data (Zero-Copy)
nodes_bytes = graph.to_arrow(label="Customer")
reader = pa.ipc.open_stream(nodes_bytes)
df = reader.read_next_batch().to_pandas()

# 2. Feature Engineering
features = df[['age', 'total_spend', 'visits']]
target = df['churned']

# 3. Train Model
clf = RandomForestClassifier()
clf.fit(features, target)
print("Model trained!")
```
