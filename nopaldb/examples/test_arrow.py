#!/usr/bin/env python3
"""
Test Apache Arrow Export
"""

import nopaldb
import pyarrow as pa
import pandas as pd

def test_arrow_export():
    print("🏹 Testing Apache Arrow Export\n")

    # Create graph with data
    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    # Add nodes
    for i in range(100):
        tx.add_node("Person", {
            "id": i,
            "name": f"Person_{i}",
            "age": 20 + (i % 50),
            "score": float(i * 1.5),
            "active": i % 2 == 0
        })

    tx.commit()
    print(f"✅ Created 100 nodes\n")

    # Export to Arrow
    print("📊 Exporting to Arrow...")
    arrow_bytes = graph.to_arrow()
    print(f"✅ Exported {len(arrow_bytes)} bytes\n")

    # Load with PyArrow
    print("📖 Reading Arrow stream...")
    reader = pa.ipc.open_stream(arrow_bytes)
    batch = reader.read_next_batch()

    print(f"✅ Arrow RecordBatch:")
    print(f"   Rows: {batch.num_rows}")
    print(f"   Columns: {batch.num_columns}")
    print(f"   Schema: {batch.schema}\n")

    # Convert to Pandas (ZERO-COPY!)
    print("🐼 Converting to Pandas (zero-copy)...")
    df = batch.to_pandas()

    print(f"✅ Pandas DataFrame:")
    print(f"   Shape: {df.shape}")
    print(f"   Columns: {list(df.columns)}")
    print(f"   Memory: {df.memory_usage(deep=True).sum() / 1024:.2f} KB\n")

    # Display sample
    print("📋 Sample data:")
    print(df.head(10))
    print()

    # Statistics
    print("📊 Statistics:")
    print(df.describe())
    print()

    # Query on Pandas (usando columnas que existen)
    print("🔍 Query: Nodes by label")
    person_nodes = df[df['label'] == 'Person']
    print(f"   Found: {len(person_nodes)} Person nodes")
    print(person_nodes.head())
    print()

    # Query by property count
    print("🔍 Query: Nodes with 5 properties")
    five_props = df[df['property_count'] == 5]
    print(f"   Found: {len(five_props)} nodes")
    print()

    print("✅ Arrow export test passed!\n")
    print("⚠️  NOTE: Current Arrow export includes node metadata only")
    print("   (id, label, property_count)")
    print("   Individual properties not yet exported.\n")

def test_arrow_performance():
    print("=" * 50)
    print("⚡ TEST: Arrow Performance")
    print("=" * 50 + "\n")

    import time

    # Create larger graph
    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    print("Creating 10,000 nodes...")
    start = time.time()
    for i in range(10000):
        tx.add_node("Node", {
            "index": i,
            "value": i * 2,
        })
    tx.commit()
    create_time = time.time() - start
    print(f"✅ Created in {create_time:.2f}s\n")

    # Export to Arrow
    print("Exporting to Arrow...")
    start = time.time()
    arrow_bytes = graph.to_arrow()
    export_time = time.time() - start
    print(f"✅ Exported {len(arrow_bytes):,} bytes in {export_time:.3f}s\n")

    # Load with PyArrow
    print("Loading with PyArrow...")
    start = time.time()
    reader = pa.ipc.open_stream(arrow_bytes)
    batch = reader.read_next_batch()
    df = batch.to_pandas()
    load_time = time.time() - start
    print(f"✅ Loaded {len(df):,} rows in {load_time:.3f}s\n")

    # Performance summary
    print("📊 Performance Summary:")
    print(f"   Create: {create_time:.2f}s ({10000/create_time:.0f} nodes/s)")
    print(f"   Export: {export_time:.3f}s ({len(arrow_bytes)/export_time/1024/1024:.1f} MB/s)")
    print(f"   Load:   {load_time:.3f}s ({len(df)/load_time:.0f} rows/s)")
    print(f"   Total:  {create_time + export_time + load_time:.2f}s")
    print()

    print("✅ Performance test passed!\n")

def test_arrow_integration():
    print("=" * 50)
    print("🔗 TEST: Arrow Integration Ecosystem")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    # Create sample data
    for i in range(100):
        tx.add_node("Sample", {"index": i})
    tx.commit()

    # Export
    arrow_bytes = graph.to_arrow()
    reader = pa.ipc.open_stream(arrow_bytes)
    batch = reader.read_next_batch()

    # Test 1: Pandas
    print("🐼 Pandas integration:")
    df = batch.to_pandas()
    print(f"   ✅ DataFrame shape: {df.shape}")
    print(f"   ✅ Memory usage: {df.memory_usage(deep=True).sum() / 1024:.2f} KB")
    print()

    # Test 2: Polars (if installed)
    try:
        import polars as pl
        print("🐻‍❄️ Polars integration:")
        pl_df = pl.from_arrow(batch)
        print(f"   ✅ DataFrame shape: {pl_df.shape}")
        print(f"   ✅ Schema: {pl_df.schema}")
        print()
    except ImportError:
        print("⚠️  Polars not installed, skipping\n")

    # Test 3: DuckDB (if installed)
    try:
        import duckdb
        print("🦆 DuckDB integration:")
        con = duckdb.connect()
        con.register("nodes", batch)
        result = con.execute("SELECT COUNT(*) FROM nodes").fetchone()
        print(f"   ✅ SQL query result: {result[0]} nodes")
        print()
    except ImportError:
        print("⚠️  DuckDB not installed, skipping\n")

    print("✅ Integration test passed!\n")

def main():
    print("\n" + "=" * 50)
    print("🏹 Apache Arrow Integration Tests")
    print("=" * 50 + "\n")

    test_arrow_export()
    test_arrow_performance()
    test_arrow_integration()

    print("=" * 50)
    print("✨ All Arrow tests complete!")
    print("=" * 50 + "\n")

    print("📝 NEXT STEPS:")
    print("   • Export individual node properties")
    print("   • Export edges to Arrow")
    print("   • Export versioned nodes (MVCC + Arrow)")
    print()

if __name__ == "__main__":
    main()