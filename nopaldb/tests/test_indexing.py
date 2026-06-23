#!/usr/bin/env python3
# examples/test_indexing.py

import nopaldb
import time

# Crear base de datos
graph = nopaldb.Graph.in_memory()

print("📊 Measurement: Con y sin índices\n")

# Insertar 10K nodos
print("1️⃣ Insertando 10,000 personas...")
for i in range(10000):
    graph.execute_nql(f"""
        create (p:Person {{
            name: 'Person_{i}',
            age: {20 + (i % 50)},
            city: 'City_{i % 100}'
        }})
    """)
print("✅ Insertados\n")

# Búsqueda SIN índice
print("2️⃣ Búsqueda SIN índice (escaneo completo):")
start = time.time()
result = graph.execute_nql("""
    find n.name
    from (n:Person)
    where n.name = 'Person_5000'
""")
elapsed_no_index = time.time() - start
print(f"   ⏱️  Tiempo: {elapsed_no_index*1000:.2f}ms\n")

# Crear índice
print("3️⃣ Creando índice en Person.name...")
start = time.time()
index_name = graph.create_index("Person", "name", "hash")
elapsed_create = time.time() - start
print(f"   ✅ Índice creado: {index_name}")
print(f"   ⏱️  Tiempo de creación: {elapsed_create*1000:.2f}ms\n")

# Búsqueda CON índice
print("4️⃣ Búsqueda CON índice (hash lookup):")
start = time.time()
result = graph.execute_nql("""
    find n.name
    from (n:Person)
    where n.name = 'Person_5000'
""")
elapsed_with_index = time.time() - start
print(f"   ⏱️  Tiempo: {elapsed_with_index*1000:.2f}ms\n")

# Resultado
speedup = elapsed_no_index / elapsed_with_index
print(f"📈 RESULTADO:")
print(f"   Sin índice:  {elapsed_no_index*1000:.2f}ms")
print(f"   Con índice:  {elapsed_with_index*1000:.2f}ms")
print(f"   🚀 SPEEDUP: {speedup:.1f}x más rápido\n")

# Crear índice BTree para rangos
print("5️⃣ Creando índice BTree en Person.age...")
graph.create_index("Person", "age", "btree")

# Range query
print("6️⃣ Búsqueda por rango (age > 50):")
start = time.time()
result = graph.execute_nql("""
    find count(n) as total
    from (n:Person)
    where n.age > 50
""")
elapsed_range = time.time() - start
print(f"   ⏱️  Tiempo: {elapsed_range*1000:.2f}ms")
for row in result:
    print(f"   📊 Encontrados: {row.get('total')} personas\n")

# Full-text index
print("7️⃣ Creando índice Full-Text en Person.bio...")
graph.execute_nql("create (p:Person {name: 'Alice', bio: 'Expert in fraud detection'})")
graph.execute_nql("create (p:Person {name: 'Bob', bio: 'Specialist in machine learning'})")
graph.create_index("Person", "bio", "fulltext")

print("8️⃣ Búsqueda full-text:")
result = graph.execute_nql("""
    find n.name
    from (n:Person)
    where fulltext(n.bio, 'fraud detection')
""")
for row in result:
    print(f"   🔍 Encontrado: {row.get('name')}\n")

# Listar índices
print("9️⃣ Índices activos:")
indexes = graph.list_indexes()
for name, label, prop, idx_type in indexes:
    print(f"   📇 {name}: {label}.{prop} [{idx_type}]")

print("\n✅ Demo completa!")