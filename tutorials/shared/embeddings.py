"""Helper para generar y persistir embeddings con sentence-transformers.

Política híbrida:
- Si existe `precomputed/<cache_name>.npz` → cargar (rápido, offline).
- Si no → computar con el modelo y persistir para próximas corridas.
- CI debe partir del .npz pre-generado para no descargar modelos.

Usado por Acto 2 (Synthetic Offshore Network) y Acto 4 (Synthetic Fraud).
"""

from __future__ import annotations

import hashlib
import warnings
from pathlib import Path
from collections import defaultdict, deque
from typing import Iterable, List, Tuple

import numpy as np

from .paths import PRECOMPUTED_DIR


_DEFAULT_MODEL = "all-MiniLM-L6-v2"
_FALLBACK_DIM = 384  # dimensión de all-MiniLM-L6-v2


def _deterministic_fallback(texts: List[str]) -> np.ndarray:
    """Embeddings deterministas (hash-seeded) para correr offline sin modelo.

    Ilustrativos, NO semánticos: permiten que los Actos 2 y 4 se ejecuten con
    `make install-min` (sin sentence-transformers) cuando no hay un `.npz`
    cacheado que coincida. Para embeddings reales usa `make -C tutorials install`.
    """
    out = np.empty((len(texts), _FALLBACK_DIM), dtype=np.float32)
    for i, text in enumerate(texts):
        seed = int(hashlib.sha256(text.encode("utf-8")).hexdigest()[:8], 16)
        out[i] = np.random.RandomState(seed).normal(0, 1, _FALLBACK_DIM).astype(np.float32)
    return out


def _ensure_dir() -> None:
    PRECOMPUTED_DIR.mkdir(parents=True, exist_ok=True)


def encode_texts(
    texts: List[str],
    cache_name: str,
    model_name: str = _DEFAULT_MODEL,
    force_recompute: bool = False,
) -> np.ndarray:
    """Devuelve matriz (N, dim) de embeddings. Persiste en precomputed/.

    Si `cache_name.npz` existe y `force_recompute=False`, se reusa siempre que
    el conjunto de textos coincida en orden y tamaño con el cacheado.
    """
    _ensure_dir()
    cache_path = PRECOMPUTED_DIR / f"{cache_name}.npz"

    if cache_path.is_file() and not force_recompute:
        with np.load(cache_path, allow_pickle=True) as data:
            cached_texts = list(data["texts"])
            cached_vectors = data["vectors"].astype(np.float32)
            if cached_texts == list(texts):
                return cached_vectors
            if sorted(cached_texts) == sorted(texts):
                by_text = defaultdict(deque)
                for text, vector in zip(cached_texts, cached_vectors):
                    by_text[text].append(vector)
                return np.array(
                    [by_text[text].popleft() for text in texts],
                    dtype=np.float32,
                )

    # Import perezoso: sentence-transformers solo se requiere al recomputar.
    try:
        from sentence_transformers import SentenceTransformer
    except ModuleNotFoundError:
        # Sin sentence-transformers y sin cache válido: degradación determinista
        # para que los tutoriales corran offline. No se persiste el .npz para no
        # contaminar el cache real si más tarde se instala el modelo.
        warnings.warn(
            f"sentence-transformers no está instalado y no hay cache para "
            f"'{cache_name}'. Usando embeddings deterministas ilustrativos; "
            f"instala con `make -C tutorials install` para embeddings reales.",
            RuntimeWarning,
            stacklevel=2,
        )
        return _deterministic_fallback(texts)

    model = SentenceTransformer(model_name)
    vectors = model.encode(texts, show_progress_bar=False, convert_to_numpy=True)
    vectors = vectors.astype(np.float32)
    np.savez(cache_path, texts=np.array(texts, dtype=object), vectors=vectors)
    return vectors


def attach_node_embeddings(
    graph,
    items: Iterable[Tuple[int, str]],
    cache_name: str,
    model_label: str = "minilm",
    sentence_model: str = _DEFAULT_MODEL,
) -> int:
    """Genera embeddings para (node_id, text) y los inserta en NopalDB.

    Devuelve cuántos vectores se insertaron.
    """
    items = list(items)
    if not items:
        return 0
    if not hasattr(graph, "add_node_embedding"):
        print("Nota v0.4.19: binding Python sin add_node_embedding; se omite adjuntar embeddings.")
        return 0
    node_ids = [nid for nid, _ in items]
    texts = [txt for _, txt in items]
    vectors = encode_texts(texts, cache_name=cache_name, model_name=sentence_model)
    for nid, vec in zip(node_ids, vectors):
        graph.add_node_embedding(nid, vec.tolist(), model_label)
    return len(items)
