# examples/mobile/on_device_analytics.py

"""
On-Device Graph Analytics con NopalDB + PyG

Este ejemplo muestra cómo ejecutar analytics y ML
100% localmente en un dispositivo móvil.

Requirements:
- pyarrow
- torch
- torch-geometric
- numpy
- sklearn
"""

import torch
from torch_geometric.nn import GCNConv
import pyarrow.parquet as pq
import numpy as np
from sklearn.cluster import KMeans

class MobileEmbedder(torch.nn.Module):
    """Lightweight GNN for mobile devices"""

    def __init__(self, num_features=1, embedding_dim=128):
        super(MobileEmbedder, self).__init__()
        self.conv1 = GCNConv(num_features, 64)
        self.conv2 = GCNConv(64, embedding_dim)

    def forward(self, x, edge_index):
        x = self.conv1(x, edge_index)
        x = torch.relu(x)
        x = self.conv2(x, edge_index)
        return x

class OnDeviceAnalytics:
    """Complete on-device analytics suite"""

    def __init__(self, parquet_path):
        self.parquet_path = parquet_path
        self.embedder = None
        self.embeddings = None
        self.num_nodes = 0
        self.node_features = None
        self.edge_index = None

    def load_graph(self):
        """Load graph from NopalDB export"""
        print("📱 Loading graph from local storage...")

        table = pq.read_table(self.parquet_path)
        df = table.to_pandas()

        self.num_nodes = len(df)
        self.node_features = torch.tensor(
            df['property_count'].values,
            dtype=torch.float
        ).reshape(-1, 1)

        # In production, export real edges from NopalDB
        self.edge_index = self._create_edges(self.num_nodes)

        print(f"   ✅ Loaded {self.num_nodes} nodes")
        return self

    def _create_edges(self, num_nodes):
        """Create sample edges"""
        num_edges = num_nodes * 3
        return torch.tensor([
            [np.random.randint(0, num_nodes) for _ in range(num_edges)],
            [np.random.randint(0, num_nodes) for _ in range(num_edges)]
        ], dtype=torch.long)

    def load_model(self, model_path):
        """Load pretrained embedder"""
        print("📦 Loading model...")

        self.embedder = MobileEmbedder()
        self.embedder.load_state_dict(
            torch.load(model_path, map_location='cpu')
        )
        self.embedder.eval()

        print("   ✅ Model loaded")
        return self

    def generate_embeddings(self):
        """Generate embeddings"""
        print("🧠 Generating embeddings...")

        with torch.no_grad():
            self.embeddings = self.embedder(
                self.node_features,
                self.edge_index
            )

        print(f"   ✅ Generated {self.embeddings.shape}")
        return self

    def find_similar(self, node_id, top_k=5):
        """Find similar nodes"""
        emb_norm = torch.nn.functional.normalize(self.embeddings, dim=1)
        query_emb = emb_norm[node_id]

        similarities = torch.matmul(emb_norm, query_emb)
        top_k_values, top_k_indices = torch.topk(similarities, k=top_k + 1)

        return [
            (int(idx), float(sim))
            for idx, sim in zip(top_k_indices[1:], top_k_values[1:])
        ]

    def cluster_nodes(self, num_clusters=3):
        """Cluster nodes"""
        print(f"🎯 Clustering into {num_clusters} groups...")

        kmeans = KMeans(n_clusters=num_clusters, random_state=42)
        labels = kmeans.fit_predict(self.embeddings.numpy())

        unique, counts = np.unique(labels, return_counts=True)
        print("\n   Cluster distribution:")
        for cluster_id, count in zip(unique, counts):
            print(f"      Cluster {cluster_id}: {count} nodes")

        return labels

    def save_embeddings(self, output_path):
        """Save embeddings"""
        np.save(output_path, self.embeddings.numpy())
        print(f"💾 Saved: {output_path}")

def main():
    print("🚀 On-Device Analytics Demo\n")
    print("=" * 50)

    # Setup
    analytics = OnDeviceAnalytics('graph_export.parquet')

    # Load
    analytics.load_graph()
    analytics.load_model('mobile_embedder.pth')

    # Generate
    analytics.generate_embeddings()

    # Find similar
    print("\n🔍 Finding similar nodes to node 0:")
    similar = analytics.find_similar(node_id=0, top_k=5)
    for idx, sim in similar:
        print(f"   Node {idx}: similarity {sim:.4f}")

    # Cluster
    print()
    labels = analytics.cluster_nodes(num_clusters=3)

    # Save
    print()
    analytics.save_embeddings('mobile_embeddings.npy')

    print("\n✅ Complete! Everything ran locally.")
    print("   No server, no internet, no cloud costs!")

if __name__ == '__main__':
    main()