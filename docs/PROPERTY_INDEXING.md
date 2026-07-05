# Implementación de Índices Secundarios (Property Indexing)

Este documento detalla la implementación del sistema de índices secundarios que permite búsquedas eficientes por propiedades en NopalDB.

## Problema

Antes de esta implementación, la función `get_node_by_property` estaba marcada como `todo!()` y no funcionaba. Para buscar un nodo por una propiedad (ej: "name" = "Alice"), habría sido necesario:

1. Obtener **todos** los nodos de la base de datos.
2. Iterar sobre cada uno verificando si tiene la propiedad deseada.

Esto es O(n) y extremadamente ineficiente para bases de datos grandes.

## Solución: Índices Invertidos

Implementamos un **índice invertido** que mapea `(propiedad, valor)` → `[lista de NodeIds]`.

### Esquema de Claves

```
idx:prop:{nombre_propiedad}:{valor} -> [NodeId, NodeId, ...]
```

**Ejemplos:**
- `idx:prop:name:Alice` → `["uuid-1234"]`
- `idx:prop:city:CDMX` → `["uuid-1234", "uuid-5678", "uuid-9abc"]`
- `idx:prop:age:30` → `["uuid-1234"]`

## Cambios Realizados

### 1. Storage Layer (`src/storage/mod.rs`)

Se añadieron dos nuevos métodos:

#### `save_property_index`
```rust
pub async fn save_property_index(
    &self, 
    property: &str, 
    value: &PropertyValue, 
    node_id: NodeId
) -> Result<()>
```

- Convierte el `PropertyValue` a string para formar la clave.
- Lee la lista existente de nodos (si hay).
- Agrega el nuevo `node_id` si no existe.
- Persiste la lista actualizada.

#### `get_nodes_by_property`
```rust
pub async fn get_nodes_by_property(
    &self, 
    property: &str, 
    value: &PropertyValue
) -> Result<Vec<NodeId>>
```

- Construye la clave del índice.
- Busca en sled y deserializa la lista de IDs.
- Retorna vector vacío si no hay coincidencias.

### 2. Graph Layer (`src/graph/mod.rs`)

#### Modificación de `add_node`

Ahora, al insertar un nodo, se indexan automáticamente todas sus propiedades:

```rust
pub async fn add_node(&self, node: Node) -> Result<NodeId> {
    // ... guardar nodo ...
    
    // Indexar propiedades (NUEVO)
    for (key, value) in &node.properties {
        self.storage.save_property_index(key, value, node_id).await?;
    }
    
    // ... resto del código ...
}
```

#### Implementación de `get_node_by_property`

```rust
pub async fn get_node_by_property(&self, property: &str, value: &str) -> Result<Node> {
    let val = PropertyValue::String(value.to_string());
    let node_ids = self.storage.get_nodes_by_property(property, &val).await?;
    
    if let Some(id) = node_ids.first() {
        self.get_node(*id).await
    } else {
        Err(NopalError::NodeNotFound(...))
    }
}
```

## Flujo de Datos

```
┌─────────────────┐
│  add_node()     │
│  (Graph)        │
└────────┬────────┘
         │
         ▼
┌─────────────────┐     ┌─────────────────┐
│ insert_node()   │     │ save_property   │
│ (Storage)       │────▶│ _index()        │
│                 │     │ (Storage)       │
└─────────────────┘     └────────┬────────┘
                                 │
                                 ▼
                        ┌─────────────────┐
                        │  sled DB        │
                        │  node:{id}      │
                        │  idx:prop:...   │
                        └─────────────────┘
```

## Limitaciones Actuales

1. **Solo búsqueda exacta**: No soporta búsquedas parciales o regex.
2. **Solo String en API**: `get_node_by_property` acepta `&str`, asume `PropertyValue::String`.
3. **No elimina índices**: `delete_node` no limpia entradas del índice (deuda técnica).
4. **Primer resultado**: Retorna solo el primer nodo que coincide, no todos.

## Tests

Se añadió el test `test_get_node_by_property` en `src/graph/mod.rs`:

```rust
#[tokio::test]
async fn test_get_node_by_property() {
    let graph = Graph::in_memory().await.unwrap();
    
    let node = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".to_string()));
        
    let node_id = graph.add_node(node).await.unwrap();
    
    // Búsqueda exitosa
    let retrieved = graph.get_node_by_property("name", "Alice").await.unwrap();
    assert_eq!(retrieved.id, node_id);
    
    // Búsqueda fallida
    let result = graph.get_node_by_property("name", "Bob").await;
    assert!(result.is_err());
}
```

## Uso

```rust
// Crear nodo con propiedades
let alice = graph.add_node(
    Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("email", PropertyValue::String("alice@example.com".into()))
).await?;

// Buscar por nombre
let node = graph.get_node_by_property("name", "Alice").await?;
println!("Encontrado: {:?}", node.id);

// Buscar por email
let node = graph.get_node_by_property("email", "alice@example.com").await?;
```

## Próximos Pasos (Mejoras Futuras)

- [ ] Soportar búsqueda por otros tipos (`Int`, `Float`).
- [ ] Implementar `get_all_nodes_by_property` para retornar todos los resultados.
- [ ] Limpiar índices en `delete_node`.
- [ ] Añadir soporte para búsquedas con operadores (`>`, `<`, `LIKE`).
