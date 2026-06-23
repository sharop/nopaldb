"""
NopalDB - Graph Database with MVCC and NQL

A high-performance graph database written in Rust with Python bindings.
"""

from .nopaldb import Graph, QueryResult, Transaction

__version__ = "0.3.5"
__all__ = ["Graph", "QueryResult", "Transaction", "__version__"]
