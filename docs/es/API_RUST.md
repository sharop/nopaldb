# Referencia de API de NopalDB

Esta referencia documenta las estructuras y funciones públicas disponibles en la librería `nopaldb`.

## Módulo `storage`

El módulo principal para interactuar con la base de datos.

### `struct Storage`

Manejador principal de la base de datos. Envuelve una instancia de `sled::Db`.

#### `Storage::new(path: &str) -> Self`
Abre o crea una base de datos en la ruta especificada.
- **Argumentos**:
    - `path`: Ruta del sistema de archivos donde se guardarán los datos.
- **Retorna**: Una nueva instancia de `Storage`.
- **Pánico**: Si no puede abrir la base de datos, el programa entrará en pánico (panic).

#### `Storage::insert_node(&self, key: &str, value: &str) -> Result<(), sled::Error>`
Inserta un par clave-valor simple.
- **Argumentos**:
    - `key`: Clave única para el nodo.
    - `value`: Valor a almacenar.
- **Retorna**: `Result` indicando éxito o error de `sled`.

#### `Storage::get_node(&self, key: &str) -> Option<String>`
Recupera el valor asociado a una clave.
- **Argumentos**:
    - `key`: Clave a buscar.
- **Retorna**: `Option<String>` con el valor si existe, o `None`.

#### `Storage::delete_node(&self, key: &str) -> Result<(), sled::Error>`
Elimina un nodo por su clave.
- **Argumentos**:
    - `key`: Clave a eliminar.
- **Retorna**: `Result` indicando éxito o error.

#### `Storage::insert_triple(&self, triple: RDFTriple) -> Result<(), sled::Error>`
Inserta una tripleta RDF en la base de datos.
- **Argumentos**:
    - `triple`: La estructura `RDFTriple` a insertar.
- **Retorna**: `Result` indicando éxito o error.
- **Detalle**: Genera una clave interna combinando Sujeto y Predicado.

#### `Storage::get_object(&self, subject: &str, predicate: &str) -> Option<String>`
Busca el objeto de una relación, dado un sujeto y un predicado.
- **Argumentos**:
    - `subject`: El nodo origen.
    - `predicate`: La relación.
- **Retorna**: `Option<String>` con el objeto (nodo destino) si existe.

---

## Módulo `transaction`

Provee mecanismos para ejecutar operaciones atómicas sobre el grafo.

### `struct Transaction`

Maneja el ciclo de vida de una transacción (ACID local).

#### `Transaction::add_node(&mut self, node: Node) -> Result<NodeId>`
Agrega un nodo al buffer de la transacción.
- **Retorna**: El ID del nodo.

#### `Transaction::get_node(&self, id: NodeId) -> Result<Node>`
Obtiene un nodo, incluyendo cambios pendientes en la transacción actual.
- **Retorna**: El nodo solicitado o error si no existe.

#### `Transaction::delete_node(&mut self, id: NodeId) -> Result<()>`
Marca un nodo para eliminación.

#### `Transaction::commit(self) -> Result<()>`
Aplica permanentemente todos los cambios pendientes al grafo.
- **Atomicidad**: Todos los cambios se aplican o ninguno.

#### `Transaction::rollback(self) -> Result<()>`
Descarta todos los cambios pendientes de la transacción.

---

## Módulo `rdf_owl::rdf`

Estructuras de datos para representación semántica.

### `struct RDFTriple`

Representa una tripleta semántica: Sujeto -> Predicado -> Objeto.

#### Campos
- `pub subject: String`
- `pub predicate: String`
- `pub object: String`

#### `RDFTriple::new(subject: &str, predicate: &str, object: &str) -> Self`
Constructor para crear una nueva tripleta.
- **Argumentos**:
    - `subject`: Identificador del sujeto.
    - `predicate`: Identificador del predicado (relación).
    - `object`: Identificador o valor del objeto.
- **Retorna**: Una nueva instancia de `RDFTriple`.
