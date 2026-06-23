#!/usr/bin/env python3
"""
Debug: Ver si el índice se está usando
"""

import nopaldb
import time

def debug_index_usage():
    """Ver logs del executor"""
    print("=" * 70)
    print("🔍 DEBUG: Index Usage")
    print("=" * 70)

    # Enable logging
    import logging
    logging.basicConfig(level=logging.DEBUG)

    with nopaldb.Graph.open("test_dbs/synthetic_character_network.db") as graph:
        # Ensure index exists
        try:
            graph.execute_nql("drop index Character_house")
        except:
            pass

        graph.execute_nql("create index on Character(house) type hash")
        print("✅ Index created: Character_house")
        print()

        # Query that SHOULD use index
        print("Executing query that SHOULD use index:")
        print('  find c.name from (c:Character) where c.house = "TeamA" limit 10')
        print()

        start = time.time()
        result = graph.execute_nql('find c.name from (c:Character) where c.house = "TeamA" limit 10')
        rows = list(result)
        elapsed = time.time() - start

        print(f"Results: {len(rows)} rows in {elapsed*1000:.2f}ms")
        print()
        print("🔍 Check logs above for:")
        print("  - '🚀 Attempting index lookup'")
        print("  - '✅ Index returned X nodes'")
        print()
        print("If you DON'T see those logs, the index is NOT being used!")

if __name__ == "__main__":
    debug_index_usage()