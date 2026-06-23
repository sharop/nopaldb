"""
NopalDB ML Integration Demo
Zero-copy Graph to PyTorch Geometric

Usage:
    python examples/ml_demo_pyg.py
"""

import nopaldb

def main():
    print("🚀 NopalDB → PyTorch Geometric Demo\n")
    
    # 1. Create graph
    print("1. Creating graph with 100 users...")
    graph = nopaldb.Graph(":memory:")
    
    tx = graph.begin_transaction()
    
    node_ids = []
    for i in range(100):
        node_id = tx.add_node("User", {
            "id": i,
            "age": 20 + (i % 50),
            "score": float(i * 0.5),
            "active": i % 2 == 0
        })
        node_ids.append(node_id)
    
    # Add some edges
    for i in range(99):
        tx.add_edge(node_ids[i], node_ids[i+1], "FOLLOWS")
    
    # Add cross-connections
    for i in range(0, 100, 10):
        if i + 10 < 100:
            tx.add_edge(node_ids[i], node_ids[i+10], "FOLLOWS")
    
    tx.commit()
    print(f"   ✅ Created {len(node_ids)} nodes\n")
    
    # 2. Convert to PyG (TODO: implement zero-copy)
    print("2. Converting to PyTorch Geometric format...")
    print("   ⚠️  NOTE: Zero-copy implementation in progress")
    print("   Current: Using basic conversion\n")
    
    # This will be the killer feature:
    # data = graph.to_pyg(label="User")
    
    # For now, demonstrate the API design:
    print("3. Planned API:")
    print("""
    import nopaldb
    from torch_geometric.nn import GCN
    
    # One-liner conversion (ZERO-COPY)
    data = graph.to_pyg(label="User")
    
    # Train GNN directly
    model = GCN(in_channels=3, out_channels=1)
    model.fit(data)
    """)
    
    print("\n4. Current capabilities:")
    print("   ✅ Arrow export implemented")
    print("   ✅ ML module structure created")
    print("   🔧 Zero-copy tensor conversion (in progress)")
    print("   📅 Python bindings (next step)")
    
    print("\n🎯 Killer feature timeline:")
    print("   Week 1-2: Arrow → Tensor (zero-copy)")
    print("   Week 3-4: DGL integration")
    print("   Week 5-6: performance evaluation")
    print("   Week 7-8: Documentation + Launch")

if __name__ == "__main__":
    main()
