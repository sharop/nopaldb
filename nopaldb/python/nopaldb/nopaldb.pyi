# Type stubs for the native `nopaldb.nopaldb` extension module (PyO3).
# Kept in sync with src/python/*.rs. Verified against the built extension by
# python/scripts/check_stubs.py (mypy stubtest; type-only names live in
# python/scripts/stubtest_allowlist.txt).
from typing import Any, Callable, Iterator, Optional, TypedDict, final

__version__: str
__author__: str

# Property values accepted/returned across the API. Lists, tuples and dicts
# nest (a tuple is stored as a list); None is stored as null.
Property = str | int | float | bool | bytes | None | list["Property"] | tuple["Property", ...] | dict[str, "Property"]
Props = dict[str, Property]

# What `NqlResult.write` reports for a write statement.
class WriteResult(TypedDict):
    nodes_created: int
    edges_created: int
    nodes_deleted: int
    edges_deleted: int
    nodes_updated: int
    edges_updated: int
    properties_changed: int
    created_ids: list[str]

# What `BulkLoader.finish()` returns.
class BulkLoadStats(TypedDict):
    nodes: int
    edges: int
    duration_secs: float
    nodes_per_second: float

# ── get_stats() (#158) ─────────────────────────────────────────────
# One event of the progress callback: `phase` is a stable name
# ("wal_replay", "adjacency_rebuild", "index_load", "index_build",
# "property_index_rebuild", "bulk_load", "upsert_batch"); `total` is None
# when unknown up front (a bulk load) and equals `done` on the last event.
class ProgressEvent(TypedDict):
    phase: str
    done: int
    total: int | None

ProgressCallback = Callable[[ProgressEvent], object]

class GraphStats(TypedDict):
    total_nodes: int
    total_edges: int
    avg_degree: float
    nodes_per_label: dict[str, int]
    edges_per_type: dict[str, int]

class StorageStats(TypedDict):
    engine: str            # "redb" | "sled"
    profile: str           # "default" | "mobile" | "server"
    data_dir: str | None   # None in memory
    read_only: bool

class WalStats(TypedDict):
    bytes: int                          # what the next open would replay
    checkpoint_threshold_bytes: int     # 0 = manual and close() only
    checkpoints_this_session: int
    last_checkpoint_unix_ms: int | None
    direct_write_durability: str        # "process_crash" | "immediate"

class OpenPhasesMs(TypedDict):
    storage: int
    wal_replay: int
    adjacency: int
    indexes: int
    total: int

class RecoveryStats(TypedDict):
    wal_records_read: int
    operations_replayed: int
    uncommitted_txs_discarded: int
    crash_recovery: bool
    adjacency_rebuilt: bool
    open_ms: OpenPhasesMs

class IndexStats(TypedDict):
    name: str
    label: str
    property: str
    type: str              # "Hash" | "BTree" | "FullText" | "Taxonomy"
    size: int
    analyzer: str | None   # full-text only: "default" or "spanish+stemming+…"

class HnswStats(TypedDict):
    model: str
    size: int
    tombstones: int
    dimension: int
    needs_rebuild: bool
    persisted: bool
    loaded_from_disk_ms: int | None

class GcAutoStats(TypedDict):
    interval_secs: int
    cutoff_timestamp: int
    min_versions_to_keep: int
    max_nodes_per_cycle: int
    dry_run: bool
    use_active_horizon: bool

class GcRunStats(TypedDict):
    unix_ms: int
    nodes_scanned: int
    versions_removed: int
    bytes_freed: int
    duration_ms: int
    dry_run: bool

class GcStats(TypedDict):
    auto_running: bool
    auto: GcAutoStats | None
    last_run: GcRunStats | None

class Stats(TypedDict):
    graph: GraphStats
    storage: StorageStats
    wal: WalStats
    recovery: RecoveryStats
    indexes: list[IndexStats]
    hnsw: list[HnswStats]
    gc: GcStats
    # Deprecated flat keys of 0.6.x, kept one more minor. Strings, as before.
    total_nodes: str
    total_edges: str
    avg_degree: str
    storage_engine: str
    wal_bytes: str

@final
class Graph:
    # ── Construction ────────────────────────────────────────────────
    @staticmethod
    def open(path: str) -> "Graph": ...
    @staticmethod
    def open_with_profile(path: str, profile: str = "default") -> "Graph": ...
    # `engine` is "auto" (default: the engine of the database already at
    # `path`, redb for a new one), "redb" or "sled". Availability depends on how
    # the package was BUILT: the wheels on PyPI ship both engines since 0.6.0.
    # `on_progress` receives the phases of this open (WAL replay, rebuilds)
    # and stays registered for later long operations (see set_progress_callback).
    @staticmethod
    def open_with_options(
        path: str,
        engine: str = "auto",
        profile: str = "default",
        on_progress: ProgressCallback | None = None,
    ) -> "Graph": ...
    @staticmethod
    def in_memory() -> "Graph": ...
    @staticmethod
    def in_memory_with_profile(profile: str = "default") -> "Graph": ...
    @staticmethod
    def in_memory_with_options(engine: str = "auto", profile: str = "default") -> "Graph": ...
    # Copy a closed database directory to another engine with verification;
    # returns {"keyspaces": [{"name", "pairs", "bytes"}, ...], "verified": bool}.
    @staticmethod
    def migrate(src: str, dst: str, src_engine: str = "auto", dst_engine: str = "auto", profile: str = "default") -> dict[str, Any]: ...

    # ── Queries & transactions ──────────────────────────────────────
    def execute_nql(self, query: str) -> "NqlResult": ...
    def begin_transaction(self, isolation: Optional[str] = None) -> "Transaction": ...

    # ── Idempotent upsert (M1-4) ────────────────────────────────────
    def upsert(
        self,
        label: str,
        key: str,
        props: Props,
        vector: Optional[list[float]] = None,
        model: Optional[str] = None,
        links: Optional[list[dict[str, Any]]] = None,
    ) -> tuple[str, str]: ...
    def upsert_many(self, requests: list[dict[str, Any]]) -> list[tuple[str, str]]: ...
    def delete(self, label: str, key: str, value: Property) -> Optional[str]: ...
    def search_hybrid(
        self,
        text: Optional[str] = None,
        vector: Optional[list[float]] = None,
        model: Optional[str] = None,
        k: int = 10,
        ef: Optional[int] = None,
        label: Optional[str] = None,
        props: Optional[Props] = None,
        text_index: Optional[str] = None,
        rrf_k: float = 60.0,
    ) -> list[dict[str, Any]]: ...
    # Misma búsqueda que search_hybrid, más la traza de por qué cada hit
    # quedó donde quedó: scores crudos por rama, configuración efectiva y
    # underfill. Ver docs/HYBRID_SEARCH.md.
    def search_hybrid_explain(
        self,
        text: Optional[str] = None,
        vector: Optional[list[float]] = None,
        model: Optional[str] = None,
        k: int = 10,
        ef: Optional[int] = None,
        label: Optional[str] = None,
        props: Optional[Props] = None,
        text_index: Optional[str] = None,
        rrf_k: float = 60.0,
    ) -> dict[str, Any]: ...

    # ── Schema & stats ──────────────────────────────────────────────
    # Both count storage keys without deserializing: exact, O(N), allocation-free.
    def node_count(self) -> int: ...
    def edge_count(self) -> int: ...
    def get_labels(self) -> list[str]: ...
    def get_edge_types(self) -> list[str]: ...
    def get_schema(self) -> dict[str, Any]: ...
    def get_label_properties(self, label: str) -> list[str]: ...
    def get_label_count(self, label: str) -> int: ...
    def get_edge_type_properties(self, edge_type: str) -> list[str]: ...
    def get_edge_type_count(self, edge_type: str) -> int: ...
    def rebuild_schema(self) -> None: ...
    def rebuild_indexes(self) -> int: ...
    # Operational state in one call: see docs/OPERATIONS.md. The nested
    # sections carry native types; the flat keys are the 0.6.x strings.
    def get_stats(self) -> Stats: ...
    # Progress of create_index / upsert_many / BulkLoader (and of the open, when
    # opened with on_progress). Called from a worker thread every ~1000 items or
    # ~250 ms; keep it cheap and do not call the graph from it. None removes it.
    def set_progress_callback(self, callback: ProgressCallback | None) -> None: ...

    # ── Indexes ─────────────────────────────────────────────────────
    def create_index(
        self,
        label: str,
        property: str,
        index_type: str = "hash",
        analyzer: dict[str, Any] | None = None,
    ) -> str: ...
    def drop_index(self, index_name: str) -> None: ...
    def list_indexes(self) -> list[tuple[str, str, str, str]]: ...
    def describe_index(self, index_name: str) -> dict[str, Any] | None: ...

    # ── Arrow export ────────────────────────────────────────────────
    # Arrow IPC streams (pyarrow.ipc.open_stream). `label` filters nodes and
    # adds their property columns; without it only id/label/property_count.
    # edges_to_arrow on a graph without edges is an empty batch with columns
    # id/source/target/edge_type (same schema as to_arrow_complete's).
    def to_arrow(self, label: str | None = None) -> bytes: ...
    def edges_to_arrow(self) -> bytes: ...
    def to_arrow_complete(self, label: str | None = None) -> tuple[bytes, bytes]: ...
    def bulk_loader(self, batch_size: int) -> "BulkLoader": ...

    # ── Embeddings & vector search ──────────────────────────────────
    def add_node_embedding(self, node_id: str, vector: list[float], model: str) -> None: ...
    def add_edge_embedding(self, edge_id: str, vector: list[float], model: str) -> None: ...
    def add_path_reference_embedding(
        self, name: str, node_model: str, edge_model: str, vector: list[float]
    ) -> None: ...
    def get_node_embedding(self, node_id: str, model: str) -> list[float]: ...
    def knn_nodes(self, query_vector: list[float], k: int, model: str, ef_search: Optional[int] = None) -> list[tuple[str, float]]: ...
    def embedding_index_stats(self, model: str) -> HnswStats | None: ...

    # ── Semantic layer ──────────────────────────────────────────────
    def import_turtle(self, ttl_source: str) -> dict[str, Any]: ...
    def export_turtle(self) -> tuple[str, dict[str, Any]]: ...
    def export_owl_file(self, path: str) -> dict[str, Any]: ...
    def rdf_prefixes(self) -> dict[str, str]: ...
    def validate_shapes(self, shapes_turtle: str) -> dict[str, Any]: ...

    # ── Lifecycle ───────────────────────────────────────────────────
    def storage_engine(self) -> str: ...
    def checkpoint(self) -> None: ...
    def close(self) -> None: ...
    def __enter__(self) -> "Graph": ...
    def __exit__(self, exc_type: Any, exc_value: Any, traceback: Any) -> bool: ...
    def __repr__(self) -> str: ...

@final
class Transaction:
    def add_node(self, label: str, properties: Props) -> str: ...
    def add_edge(
        self, source: str, target: str, edge_type: str, properties: Optional[Props] = None
    ) -> str: ...
    def commit(self) -> None: ...
    def rollback(self) -> None: ...
    def __repr__(self) -> str: ...

@final
class QueryResult:
    @property
    def columns(self) -> list[str]: ...
    def __len__(self) -> int: ...
    def __iter__(self) -> Iterator[dict[str, Any]]: ...
    def __getitem__(self, index: int, /) -> dict[str, Any]: ...
    def __repr__(self) -> str: ...

@final
class ProfileResult:
    @property
    def plan(self) -> str: ...
    @property
    def statement_type(self) -> str: ...
    @property
    def execution_ms(self) -> float: ...
    @property
    def rows_returned(self) -> int: ...
    @property
    def columns(self) -> list[str]: ...
    @property
    def path_query(self) -> bool: ...
    @property
    def path_metrics(self) -> Optional[Any]: ...
    def __repr__(self) -> str: ...

@final
class NqlResult:
    @property
    def kind(self) -> str: ...
    @property
    def summary(self) -> str: ...
    @property
    def query(self) -> Optional[QueryResult]: ...
    @property
    def write(self) -> Optional[WriteResult]: ...
    @property
    def explain(self) -> Optional[str]: ...
    @property
    def profile(self) -> Optional[ProfileResult]: ...
    @property
    def message(self) -> Optional[str]: ...
    def __len__(self) -> int: ...
    def __iter__(self) -> Iterator[dict[str, Any]]: ...
    def __getitem__(self, index: int, /) -> dict[str, Any]: ...
    def __repr__(self) -> str: ...

# graph.bulk_loader(batch_size). Property values as in Transaction (same
# converter): None is null, bytes/lists/dicts round-trip. Use as a context
# manager; finish() runs on exit.
@final
class BulkLoader:
    def add_node(self, label: str, properties: Props) -> str: ...
    def add_edge(
        self, source: str, target: str, edge_type: str, properties: Optional[Props] = None
    ) -> str: ...
    def finish(self) -> BulkLoadStats: ...
    def __enter__(self) -> "BulkLoader": ...
    def __exit__(self, exc_type: Any, exc_value: Any, traceback: Any) -> bool: ...
    def __repr__(self) -> str: ...

# ── Semantic reasoner (feature `python-reasoner`) ───────────────────
# In a build without the feature the `nopaldb` package sets both names to
# None (see __init__.py); the wheels on PyPI have them.
@final
class Inference:
    @property
    def sub(self) -> str: ...
    @property
    def super_class(self) -> str: ...
    @property
    def rule(self) -> str: ...
    def to_dict(self) -> dict[str, Any]: ...
    def __repr__(self) -> str: ...

@final
class ELReasoner:
    def __init__(self) -> None: ...
    def register_class(self, node_id: str, label: str) -> None: ...
    def assert_subclass(self, sub: str, super_class: str) -> list[Inference]: ...
    # left ⊓ right ⊑ result
    def assert_conjunction(self, left: str, right: str, result: str) -> list[Inference]: ...
    def assert_existential(self, sub: str, role: str, filler: str) -> list[Inference]: ...
    # ∃role.filler ⊑ result
    def assert_existential_domain(self, role: str, filler: str, result: str) -> list[Inference]: ...
    def classify_all(self) -> list[Inference]: ...
    def is_subclass_of(self, sub: str, super_class: str) -> bool: ...
    def superclasses(self, node: str) -> list[str]: ...
    def subclasses(self, node: str) -> list[str]: ...
    def axiom_count(self) -> int: ...
    def derived_count(self) -> int: ...
    def derived_inferences(self) -> list[Inference]: ...
    def __repr__(self) -> str: ...
