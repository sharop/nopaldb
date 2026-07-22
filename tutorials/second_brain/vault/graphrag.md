# GraphRAG

## Beyond flat retrieval

Classic RAG retrieves text chunks by vector similarity alone. GraphRAG adds the
relationships between chunks, so retrieval can follow edges — citations, links,
shared entities — instead of treating every chunk as isolated.

## Building blocks

GraphRAG needs a store that holds both vectors and a traversable graph. It draws
on [[knowledge-graphs]] for structure and [[neuro-symbolic]] retrieval to fuse
similarity with those edges.
