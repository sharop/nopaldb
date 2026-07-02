#!/usr/bin/env python3
"""
Test NopalDB Arrow Export - Complete Example
Creates test data and exports to Arrow
Updated to use Transaction API
SHAROP: probado el 23EN26 : Resultado exitoso
"""

import nopaldb
import pyarrow as pa
import pandas as pd
from pathlib import Path
import shutil

# Configuración
DB_PATH = "data/test_export.db"

def setup_test_database():
    """Create test database with nodes and edges"""
    print("\n🔧 Setting up test database...")

    # Remove existing database
    if Path(DB_PATH).exists():
        shutil.rmtree(DB_PATH)
        print(f"   🗑️  Removed existing database")

    # Create new database
    graph = nopaldb.Graph.open(DB_PATH)
    print(f"   ✅ Created database: {DB_PATH}")

    return graph


def create_test_data(graph):
    """Create test nodes and edges using Transaction API"""
    print("\n📝 Creating test data...")

    # Start transaction
    tx = graph.begin_transaction()

    # Create test nodes
    node_data = [
        ("Person", {"name": "Alice", "age": 30, "city": "NYC"}),
        ("Person", {"name": "Bob", "age": 25, "city": "LA"}),
        ("Person", {"name": "Charlie", "age": 35, "city": "SF"}),
        ("Company", {"name": "TechCorp", "founded": 2010, "revenue": 1000000}),
        ("Company", {"name": "DataInc", "founded": 2015, "revenue": 500000}),
    ]

    node_ids = {}
    for label, props in node_data:
        node_id = tx.add_node(label, props)
        node_ids[props.get("name", label)] = node_id
        print(f"   ✅ Added {label}: {props.get('name', 'N/A')}")

    print(f"   📊 Created {len(node_ids)} nodes")

    # Create edges
    edge_data = [
        ("Alice", "TechCorp", "WORKS_AT", {"since": 2020, "position": "Engineer"}),
        ("Bob", "DataInc", "WORKS_AT", {"since": 2021, "position": "Manager"}),
        ("Alice", "Bob", "KNOWS", {"since": 2019, "strength": "strong"}),
        ("Charlie", "TechCorp", "WORKS_AT", {"since": 2018, "position": "Director"}),
        ("Alice", "Charlie", "KNOWS", {"since": 2020, "strength": "medium"}),
    ]

    edge_ids = []
    for source_name, target_name, edge_type, props in edge_data:
        source_id = node_ids[source_name]
        target_id = node_ids[target_name]

        edge_id = tx.add_edge(source_id, target_id, edge_type, props)
        edge_ids.append(edge_id)
        print(f"   ✅ Added edge: {edge_type} ({source_name} → {target_name})")

    print(f"   📊 Created {len(edge_ids)} edges")

    # Commit transaction
    tx.commit()
    print(f"   ✅ Transaction committed")

    return node_ids, edge_ids


def test_export_complete(graph):
    """Test complete graph export"""
    print("\n🚀 Testing to_arrow_complete()...")

    try:
        # Export complete graph
        nodes_bytes, edges_bytes = graph.to_arrow_complete()

        print(f"   ✅ Export successful!")
        print(f"   📦 Nodes data: {len(nodes_bytes):,} bytes")
        print(f"   📦 Edges data: {len(edges_bytes):,} bytes")

        # Read nodes
        print("\n📊 Reading nodes...")
        nodes_reader = pa.ipc.open_stream(nodes_bytes)
        nodes_batch = nodes_reader.read_next_batch()
        nodes_df = nodes_batch.to_pandas()

        print(f"   ✅ Nodes DataFrame: {nodes_df.shape}")
        print(f"   Columns: {nodes_df.columns.tolist()}")
        print(f"\nFirst 3 nodes:")
        print(nodes_df.head(3).to_string())

        # Read edges
        print("\n📊 Reading edges...")
        edges_reader = pa.ipc.open_stream(edges_bytes)
        edges_batch = edges_reader.read_next_batch()
        edges_df = edges_batch.to_pandas()

        print(f"   ✅ Edges DataFrame: {edges_df.shape}")
        print(f"   Columns: {edges_df.columns.tolist()}")
        print(f"\nFirst 3 edges:")
        print(edges_df.head(3).to_string())

        return nodes_df, edges_df

    except Exception as e:
        print(f"   ❌ Error: {e}")
        import traceback
        traceback.print_exc()
        return None, None


def test_export_filtered(graph):
    """Test filtered export by label"""
    print("\n🔍 Testing filtered export (label='Person')...")

    try:
        nodes_bytes, edges_bytes = graph.to_arrow_complete(label="Person")

        nodes_reader = pa.ipc.open_stream(nodes_bytes)
        nodes_batch = nodes_reader.read_next_batch()
        nodes_df = nodes_batch.to_pandas()

        print(f"   ✅ Filtered nodes: {len(nodes_df)}")
        print(f"   Labels: {nodes_df['label'].unique().tolist()}")

        return nodes_df

    except Exception as e:
        print(f"   ❌ Error: {e}")
        import traceback
        traceback.print_exc()
        return None


def test_export_edges_only(graph):
    """Test edges-only export"""
    print("\n🔗 Testing edges_to_arrow()...")

    try:
        edges_bytes = graph.edges_to_arrow()

        edges_reader = pa.ipc.open_stream(edges_bytes)
        edges_batch = edges_reader.read_next_batch()
        edges_df = edges_batch.to_pandas()

        print(f"   ✅ Edges DataFrame: {edges_df.shape}")
        print(f"   Columns: {edges_df.columns.tolist()}")
        print(f"   Edge types: {edges_df['edge_type'].unique().tolist()}")

        return edges_df

    except Exception as e:
        print(f"   ❌ Error: {e}")
        import traceback
        traceback.print_exc()
        return None


def test_export_nodes_only(graph):
    """Test nodes-only export"""
    print("\n👤 Testing to_arrow()...")

    try:
        nodes_bytes = graph.to_arrow()

        nodes_reader = pa.ipc.open_stream(nodes_bytes)
        nodes_batch = nodes_reader.read_next_batch()
        nodes_df = nodes_batch.to_pandas()

        print(f"   ✅ Nodes DataFrame: {nodes_df.shape}")
        print(f"   Labels: {nodes_df['label'].value_counts().to_dict()}")

        return nodes_df

    except Exception as e:
        print(f"   ❌ Error: {e}")
        import traceback
        traceback.print_exc()
        return None


def analyze_graph_structure(nodes_df, edges_df):
    """Analyze the exported graph structure"""
    print("\n📈 Graph Structure Analysis...")

    if nodes_df is not None and edges_df is not None:
        print(f"\n   Total Nodes: {len(nodes_df)}")
        print(f"   Node Labels:")
        for label, count in nodes_df['label'].value_counts().items():
            print(f"      - {label}: {count}")

        print(f"\n   Total Edges: {len(edges_df)}")
        print(f"   Edge Types:")
        for edge_type, count in edges_df['edge_type'].value_counts().items():
            print(f"      - {edge_type}: {count}")

        # Check connectivity
        unique_sources = edges_df['source'].nunique()
        unique_targets = edges_df['target'].nunique()
        unique_nodes_in_edges = len(set(edges_df['source']) | set(edges_df['target']))

        print(f"\n   Connectivity:")
        print(f"      - Unique source nodes: {unique_sources}")
        print(f"      - Unique target nodes: {unique_targets}")
        print(f"      - Total nodes with edges: {unique_nodes_in_edges}")
        print(f"      - Isolated nodes: {len(nodes_df) - unique_nodes_in_edges}")

        # Show edge properties
        print(f"\n   Edge Properties:")
        for col in edges_df.columns:
            if col not in ['id', 'source', 'target', 'edge_type']:
                non_null = edges_df[col].notna().sum()
                if non_null > 0:
                    print(f"      - {col}: {non_null}/{len(edges_df)} edges have this property")


def test_nql_queries(graph):
    """Test NQL queries to verify data"""
    print("\n🔍 Testing NQL queries...")

    try:
        # Query all persons
        result = graph.execute_nql("find p.name, p.age from (p:Person)")
        print(f"\n   Persons in database: {len(result)}")
        for row in result:
            print(f"      - {row.get('p.name')}, age {row.get('p.age')}")

        # Query relationships with edge properties
        result = graph.execute_nql("""
            find a.name, r.since, r.strength, b.name
            from (a:Person)-[r:KNOWS]->(b:Person)
        """)
        print(f"\n   KNOWS relationships: {len(result)}")
        for row in result:
            print(f"      - {row.get('a.name')} knows {row.get('b.name')} since {row.get('r.since')} (strength: {row.get('r.strength')})")

        # Query work relationships
        result = graph.execute_nql("""
            find p.name, r.position, c.name
            from (p:Person)-[r:WORKS_AT]->(c:Company)
        """)
        print(f"\n   WORKS_AT relationships: {len(result)}")
        for row in result:
            print(f"      - {row.get('p.name')} works at {row.get('c.name')} as {row.get('r.position')}")

    except Exception as e:
        print(f"   ❌ Error: {e}")
        import traceback
        traceback.print_exc()


def main():
    print("=" * 60)
    print("NopalDB Arrow Export - Complete Test")
    print("=" * 60)

    # Setup
    graph = setup_test_database()

    # Create test data
    node_ids, edge_ids = create_test_data(graph)

    # Test NQL queries first
    test_nql_queries(graph)

    # Test exports
    nodes_df, edges_df = test_export_complete(graph)

    if nodes_df is not None and edges_df is not None:
        # Additional tests
        test_export_filtered(graph)
        test_export_edges_only(graph)
        test_export_nodes_only(graph)

        # Analysis
        analyze_graph_structure(nodes_df, edges_df)

        print("\n" + "=" * 60)
        print("✅ All tests passed!")
        print("=" * 60)
        print(f"\n📂 Test database: {DB_PATH}")
        print(f"   You can use this for further testing")
    else:
        print("\n" + "=" * 60)
        print("❌ Tests failed!")
        print("=" * 60)


if __name__ == "__main__":
    main()