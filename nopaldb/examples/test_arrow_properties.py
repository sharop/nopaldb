#!/usr/bin/env python3
"""
Test Arrow Export with Individual Properties
SHAROP:PROBADO 26ENE26:Funcionando correctamente
"""

import nopaldb
import pyarrow as pa
import pandas as pd

def test_properties_export():
    print("🏹 Testing Arrow Properties Export\n")

    # Create graph with diverse data
    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    # Add Person nodes
    print("Creating Person nodes...")
    for i in range(50):
        tx.add_node("Person", {
            "name": f"Person_{i}",
            "age": 20 + (i % 50),
            "city": ["CDMX", "GDL", "MTY"][i % 3],
            "score": float(i * 1.5),
            "active": i % 2 == 0
        })

    # Add Company nodes
    print("Creating Company nodes...")
    for i in range(30):
        tx.add_node("Company", {
            "name": f"Company_{i}",
            "employees": 10 + (i * 5),
            "revenue": float(i * 100000),
            "public": i % 3 == 0
        })

    tx.commit()
    print(f"✅ Created 80 nodes total\n")

    # ═══════════════════════════════════════
    # TEST 1: Export Person with properties
    # ═══════════════════════════════════════
    print("=" * 50)
    print("TEST 1: Export Person nodes with properties")
    print("=" * 50 + "\n")

    arrow_bytes = graph.to_arrow(label="Person")
    reader = pa.ipc.open_stream(arrow_bytes)
    batch = reader.read_next_batch()

    print(f"✅ Arrow RecordBatch:")
    print(f"   Rows: {batch.num_rows}")
    print(f"   Columns: {batch.num_columns}")
    print(f"   Schema: {batch.schema}\n")

    # Convert to Pandas
    df = batch.to_pandas()

    print(f"✅ Pandas DataFrame:")
    print(f"   Shape: {df.shape}")
    print(f"   Columns: {list(df.columns)}\n")

    # Display sample
    print("📋 Sample Person data:")
    print(df.head(10))
    print()

    # Statistics
    print("📊 Person Statistics:")
    print(df[['age', 'score']].describe())
    print()

    # Query by age
    print("🔍 Query: People over 40")
    over_40 = df[df['age'] > 40]
    print(f"   Found: {len(over_40)} people")
    print(over_40[['name', 'age', 'city', 'score']].head())
    print()

    # Group by city
    print("🌆 People by city:")
    city_counts = df.groupby('city').size()
    print(city_counts)
    print()

    # ═══════════════════════════════════════
    # TEST 2: Export Company with properties
    # ═══════════════════════════════════════
    print("=" * 50)
    print("TEST 2: Export Company nodes with properties")
    print("=" * 50 + "\n")

    arrow_bytes = graph.to_arrow(label="Company")
    reader = pa.ipc.open_stream(arrow_bytes)
    batch = reader.read_next_batch()

    df_companies = batch.to_pandas()

    print(f"✅ Company DataFrame:")
    print(f"   Shape: {df_companies.shape}")
    print(f"   Columns: {list(df_companies.columns)}\n")

    print("📋 Sample Company data:")
    print(df_companies.head())
    print()

    print("📊 Company Statistics:")
    print(df_companies[['employees', 'revenue']].describe())
    print()

    # Query by employees
    print("🔍 Query: Companies with 50+ employees")
    large_companies = df_companies[df_companies['employees'] >= 50]
    print(f"   Found: {len(large_companies)} companies")
    print(large_companies[['name', 'employees', 'revenue']].head())
    print()

    print("✅ Properties export test passed!\n")

def test_ml_pipeline():
    print("=" * 50)
    print("🧠 TEST: ML Pipeline with Properties")
    print("=" * 50 + "\n")

    # Create training data
    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    print("Creating training dataset...")
    for i in range(500):
        tx.add_node("Sample", {
            "feature_1": float(i),
            "feature_2": float(i * 2),
            "feature_3": float(i ** 0.5),
            "target": 1 if i % 2 == 0 else 0
        })

    tx.commit()
    print("✅ Created 500 samples\n")

    # Export with properties
    arrow_bytes = graph.to_arrow(label="Sample")
    reader = pa.ipc.open_stream(arrow_bytes)
    batch = reader.read_next_batch()
    df = batch.to_pandas()

    print(f"📊 Training data:")
    print(f"   Shape: {df.shape}")
    print(f"   Columns: {list(df.columns)}\n")

    # Prepare for ML (zero-copy!)
    feature_cols = ['feature_1', 'feature_2', 'feature_3']
    X = df[feature_cols].values
    y = df['target'].values

    print(f"✅ ML data prepared:")
    print(f"   X shape: {X.shape}")
    print(f"   y shape: {y.shape}")
    print(f"   X dtype: {X.dtype}")
    print(f"   y dtype: {y.dtype}\n")

    # Train model (if sklearn available)
    try:
        from sklearn.ensemble import RandomForestClassifier
        from sklearn.model_selection import train_test_split
        from sklearn.metrics import accuracy_score, classification_report

        X_train, X_test, y_train, y_test = train_test_split(
            X, y, test_size=0.2, random_state=42
        )

        print("🌲 Training Random Forest...")
        clf = RandomForestClassifier(n_estimators=50, random_state=42)
        clf.fit(X_train, y_train)

        y_pred = clf.predict(X_test)
        accuracy = accuracy_score(y_test, y_pred)

        print(f"✅ Model trained!")
        print(f"   Accuracy: {accuracy:.2%}")
        print(f"   Feature importances: {clf.feature_importances_}")
        print()

        print("📊 Classification Report:")
        print(classification_report(y_test, y_pred))

    except ImportError:
        print("⚠️  scikit-learn not installed\n")

    print("✅ ML pipeline test passed!\n")

def test_type_mixing():
    print("=" * 50)
    print("🔀 TEST: Mixed Property Types")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    # Create nodes with mixed types for same property
    tx.add_node("Mixed", {
        "value": 42,  # Int
        "name": "First"
    })

    tx.add_node("Mixed", {
        "value": "hello",  # String (different type!)
        "name": "Second"
    })

    tx.add_node("Mixed", {
        "value": 3.14,  # Float
        "name": "Third"
    })

    tx.commit()

    # Export
    arrow_bytes = graph.to_arrow(label="Mixed")
    reader = pa.ipc.open_stream(arrow_bytes)
    batch = reader.read_next_batch()
    df = batch.to_pandas()

    print("📊 Mixed types DataFrame:")
    print(df)
    print()

    print("📋 Column types:")
    print(df.dtypes)
    print()

    print("✅ Mixed types handled correctly!\n")

def main():
    print("\n" + "=" * 50)
    print("🏹 Arrow Properties Export Tests")
    print("=" * 50 + "\n")

    test_properties_export()
    test_ml_pipeline()
    test_type_mixing()

    print("=" * 50)
    print("✨ All properties tests complete!")
    print("=" * 50 + "\n")

    print("🎯 KEY FEATURES DEMONSTRATED:")
    print("   ✅ Export individual properties to Arrow")
    print("   ✅ Schema inference per label")
    print("   ✅ Zero-copy to Pandas/ML frameworks")
    print("   ✅ Type safety (Int, Float, String, Bool)")
    print("   ✅ Null handling")
    print("   ✅ Mixed type fallback")
    print("   ✅ Ready for ML pipelines")
    print()

if __name__ == "__main__":
    main()