"""Generador determinista de un dataset estilo Synthetic Offshore Network para el tutorial.

Crea un grafo plausible para fraud detection:
  - Jurisdictions (4): paraisos fiscales con risk score
  - Intermediaries (5): firmas que registran shell companies
  - Entities (~80): mix de OffshoreEntity y ShellCompany, con descripciones
    textuales que sirven para embeddings (sentence-transformers).
  - Officers (~50): personas que aparecen como directores/accionistas.

Edges:
  - OWNS               : Officer -> Entity   (con shares: %)
  - REGISTERED_IN      : Entity  -> Jurisdiction (con incorporation_date)
  - USES_INTERMEDIARY  : Entity  -> Intermediary
  - CONTROLS           : Entity  -> Entity   (estructura corporativa)

El generador es determinista por seed (default 42). Cualquier cambio en la
topologia debe verificarse contra el smoke gate del notebook.
"""

from __future__ import annotations

import argparse
import random
from pathlib import Path

import nopaldb


JURISDICTIONS = [
    ("BVI", "British Virgin Islands", "high"),
    ("Harbor Cay", "Harbor Cay", "high"),
    ("Seychelles", "Seychelles", "medium"),
    ("Luxembourg", "Luxembourg", "low"),
]

INTERMEDIARIES = [
    ("Atlas Fiduciary Group", "Harbor Cay", "Especialista en sociedades offshore y trusts en BVI."),
    ("Alpha Services Ltd", "BVI", "Boutique de incorporacion enfocada en estructuras BVI."),
    ("Helios Compliance", "Luxembourg", "Asesoria fiscal europea con foco luxemburgues."),
    ("Triton Wealth", "Seychelles", "Trust services y gestion patrimonial en Seychelles."),
    ("OmniCorp Registrars", "Harbor Cay", "Registro masivo de sociedades, alto volumen."),
]

# Plantillas para generar nombres + descripciones plausibles.
ENTITY_PREFIXES = [
    "Sunrise", "Bluewater", "Redmond", "Zephyr", "Onyx", "Helios",
    "Pinnacle", "Sterling", "Cascade", "Atlas", "Northern", "Phoenix",
    "Crescent", "Halcyon", "Meridian", "Vanguard", "Sapphire", "Granite",
    "Westwood", "Eastpoint", "Kingdom", "Tempest", "Aurora", "Beacon",
    "Sentinel", "Voyager", "Horizon", "Spectrum", "Cobalt", "Equinox",
]
ENTITY_SUFFIXES = [
    "Holdings", "Capital", "Trust", "Group", "International", "Partners",
    "Ventures", "Investments", "Advisors", "Resources",
]
COUNTRY_CODES = ["BVI", "Harbor Cay", "Seychelles", "Luxembourg"]

INDUSTRIES = [
    ("real estate", "Inversion en bienes raices comerciales premium."),
    ("oil and gas", "Trading de hidrocarburos y derivados."),
    ("private banking", "Banca privada y custodia patrimonial."),
    ("luxury goods", "Distribucion de bienes de lujo y arte."),
    ("art trading", "Coleccion y subasta de obras de arte."),
    ("yacht charter", "Operacion de yates y vehiculos de placer."),
    ("tech holdings", "Tenencia de participaciones en startups tecnologicas."),
    ("commodities", "Trading de metales preciosos y materias primas."),
]

OFFICER_FIRST = [
    "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Hugo",
    "Ines", "Javier", "Karen", "Liam", "Mei", "Nadia", "Oscar", "Petra",
    "Quentin", "Rosa", "Said", "Tomas", "Uma", "Viktor", "Walter", "Xinyi",
    "Yara", "Zane",
]
OFFICER_LAST = [
    "Novak", "Okafor", "Svensson", "Muller", "Petrov", "Cohen", "Tanaka",
    "Bergstrom", "Almeida", "Demir", "Andersson", "Volkov", "Ferrari",
    "Hadid", "Konstantin",
]
OFFICER_COUNTRIES = [
    "Russia", "Nigeria", "Sweden", "Germany", "Spain", "Brazil", "Japan",
    "Iran", "Italy", "Saudi Arabia", "Turkey", "Mexico", "China", "Argentina",
]


def _entity_name(rng: random.Random) -> str:
    return f"{rng.choice(ENTITY_PREFIXES)} {rng.choice(ENTITY_SUFFIXES)}"


def _description(name: str, jurisdiction: str, industry: str, blurb: str) -> str:
    return f"{name} es una entidad {industry} registrada en {jurisdiction}. {blurb}"


def generate_dataset(
    db_path: str,
    reset: bool = True,
    n_entities: int = 80,
    n_officers: int = 50,
    seed: int = 42,
) -> dict:
    """Construye el dataset.

    Returns:
        dict con conteos para validacion del gate.
    """
    rng = random.Random(seed)

    path = Path(db_path)
    if reset and path.exists():
        if path.is_dir():
            for child in path.iterdir():
                if child.is_file():
                    child.unlink()
                else:
                    for sub in child.rglob("*"):
                        if sub.is_file():
                            sub.unlink()
                    child.rmdir()
            path.rmdir()
        else:
            path.unlink()
    path.parent.mkdir(parents=True, exist_ok=True)

    print("=" * 60)
    print("Generating Synthetic Offshore Network dataset (synthetic)")
    print("=" * 60)
    print(f"DB:           {db_path}")
    print(f"seed:         {seed}")
    print(f"n_entities:   {n_entities}")
    print(f"n_officers:   {n_officers}")

    counts = {
        "jurisdictions": 0,
        "intermediaries": 0,
        "entities": 0,
        "officers": 0,
        "edges_owns": 0,
        "edges_registered_in": 0,
        "edges_uses_intermediary": 0,
        "edges_controls": 0,
    }

    with nopaldb.Graph.open(db_path) as graph:
        # Jurisdictions
        tx = graph.begin_transaction()
        jur_ids = {}
        for code, name, risk in JURISDICTIONS:
            jur_ids[code] = tx.add_node(
                "Jurisdiction",
                {"name": name, "code": code, "risk": risk},
            )
        tx.commit()
        counts["jurisdictions"] = len(JURISDICTIONS)

        # Intermediaries
        tx = graph.begin_transaction()
        inter_ids = {}
        for name, country, descr in INTERMEDIARIES:
            inter_ids[name] = tx.add_node(
                "Intermediary",
                {"name": name, "country": country, "description": descr},
            )
        tx.commit()
        counts["intermediaries"] = len(INTERMEDIARIES)

        # Officers
        tx = graph.begin_transaction()
        officer_ids = []
        used_names = set()
        for _ in range(n_officers):
            while True:
                name = f"{rng.choice(OFFICER_FIRST)} {rng.choice(OFFICER_LAST)}"
                if name not in used_names:
                    used_names.add(name)
                    break
            country = rng.choice(OFFICER_COUNTRIES)
            officer_ids.append(
                tx.add_node(
                    "Officer",
                    {"name": name, "country": country, "role": "director"},
                )
            )
        tx.commit()
        counts["officers"] = n_officers

        # Entities con descripciones para embeddings.
        tx = graph.begin_transaction()
        entity_ids = []
        entity_meta = []  # paralelo: (id, name, label, jurisdiction_code, industry, shell)
        for i in range(n_entities):
            name = _entity_name(rng)
            jur_code = rng.choices(
                COUNTRY_CODES,
                weights=[0.4, 0.35, 0.15, 0.10],
            )[0]
            industry, blurb = rng.choice(INDUSTRIES)
            shell = rng.random() < 0.35
            label = "ShellCompany" if shell else "OffshoreEntity"
            year = rng.randint(2000, 2018)
            month = rng.randint(1, 12)
            day = rng.randint(1, 28)
            description = _description(
                name, jur_code, industry, blurb
            )
            nid = tx.add_node(
                label,
                {
                    "name": f"{name} #{i:03d}",
                    "industry": industry,
                    "incorporated": f"{year:04d}-{month:02d}-{day:02d}",
                    "status": rng.choice(["active", "active", "inactive"]),
                    "shell": shell,
                    "description": description,
                },
            )
            entity_ids.append(nid)
            entity_meta.append((nid, name, label, jur_code, industry, shell))
        tx.commit()
        counts["entities"] = n_entities

        # Plantar una entidad clave: "Atlas Fiduciary Group Ltd" como cliente top de
        # AtlasFiduciary, para que el gate de HNSW (similar_to "Atlas Fiduciary Group")
        # tenga un top match deterministico.
        tx = graph.begin_transaction()
        flagship_id = tx.add_node(
            "OffshoreEntity",
            {
                "name": "Atlas Fiduciary Holdings",
                "industry": "private banking",
                "incorporated": "1986-04-17",
                "status": "active",
                "shell": False,
                "description": (
                    "Atlas Fiduciary Holdings es la holding insignia del bufete "
                    "Atlas Fiduciary Group, registrada en Harbor Cay y especializada en "
                    "estructuras offshore para banca privada."
                ),
            },
        )
        tx.commit()
        entity_ids.append(flagship_id)
        entity_meta.append((flagship_id, "Atlas Fiduciary Holdings", "OffshoreEntity", "Harbor Cay", "private banking", False))
        counts["entities"] += 1

        # Edges: REGISTERED_IN
        tx = graph.begin_transaction()
        for nid, _, _, jur_code, _, _ in entity_meta:
            tx.add_edge(
                nid,
                jur_ids[jur_code],
                "REGISTERED_IN",
                {},
            )
            counts["edges_registered_in"] += 1
        tx.commit()

        # Edges: USES_INTERMEDIARY (cada entity con 1 intermediary, ponderado)
        tx = graph.begin_transaction()
        intermediary_keys = list(inter_ids.keys())
        for nid, _, _, _, _, shell in entity_meta:
            if shell:
                # shells tienden a usar Atlas Fiduciary Group u OmniCorp
                key = rng.choices(intermediary_keys, weights=[0.45, 0.10, 0.05, 0.05, 0.35])[0]
            else:
                key = rng.choice(intermediary_keys)
            tx.add_edge(nid, inter_ids[key], "USES_INTERMEDIARY", {})
            counts["edges_uses_intermediary"] += 1
        tx.commit()

        # Edges: OWNS (Officer -> Entity, con shares%)
        tx = graph.begin_transaction()
        for nid, _, _, _, _, _ in entity_meta:
            n_owners = rng.randint(1, 3)
            owners = rng.sample(officer_ids, k=n_owners)
            shares_left = 100
            for j, oid in enumerate(owners):
                if j == len(owners) - 1:
                    shares = shares_left
                else:
                    shares = rng.randint(20, max(20, shares_left - 20 * (len(owners) - j - 1)))
                shares_left -= shares
                tx.add_edge(
                    oid,
                    nid,
                    "OWNS",
                    {"shares": float(shares)},
                )
                counts["edges_owns"] += 1
        tx.commit()

        # Edges: CONTROLS - cadena corporativa entity -> entity.
        # Plantamos primero una piramide deterministica desde el flagship
        # (Atlas Fiduciary Holdings) para que las path queries del tutorial
        # siempre tengan resultados estables sin importar el seed downstream.
        tx = graph.begin_transaction()

        # Piramide: flagship -> 3 hijos -> 2 nietos cada uno (3 + 6 = 9 edges).
        # Usamos los primeros entities deterministicamente para reproducibilidad.
        children = entity_ids[0:3]
        grandchildren = [entity_ids[3:5], entity_ids[5:7], entity_ids[7:9]]
        pyramid_amounts = [
            (flagship_id, children[0], 5_000_000.0),
            (flagship_id, children[1], 3_000_000.0),
            (flagship_id, children[2], 2_500_000.0),
            (children[0], grandchildren[0][0], 2_000_000.0),
            (children[0], grandchildren[0][1], 1_500_000.0),
            (children[1], grandchildren[1][0], 1_200_000.0),
            (children[1], grandchildren[1][1], 900_000.0),
            (children[2], grandchildren[2][0], 800_000.0),
            (children[2], grandchildren[2][1], 600_000.0),
        ]
        for parent, child, amount in pyramid_amounts:
            tx.add_edge(
                parent,
                child,
                "CONTROLS",
                {"amount": amount, "controlling_stake": True},
            )
            counts["edges_controls"] += 1

        # Cadenas adicionales aleatorias para densidad.
        n_extra = max(5, n_entities // 4)
        for _ in range(n_extra):
            parent = rng.choice(entity_ids)
            child = rng.choice(entity_ids)
            if parent == child:
                continue
            amount = round(rng.uniform(100_000, 5_000_000), 2)
            tx.add_edge(
                parent,
                child,
                "CONTROLS",
                {"amount": amount, "controlling_stake": rng.choice([True, False])},
            )
            counts["edges_controls"] += 1
        tx.commit()

        # Indices basicos
        for label, prop, itype in [
            ("OffshoreEntity", "name", "hash"),
            ("ShellCompany", "name", "hash"),
            ("Officer", "name", "hash"),
            ("Jurisdiction", "code", "hash"),
            ("Intermediary", "name", "hash"),
        ]:
            try:
                graph.execute_nql(f"create index on {label}({prop}) type {itype}")
            except Exception:
                pass

        stats = graph.get_stats()
        print(f"total_nodes: {int(stats['total_nodes'])}")
        print(f"total_edges: {int(stats['total_edges'])}")

    print("=" * 60)
    print("Done.")
    print("=" * 60)
    return counts


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Generate Synthetic Offshore Network synthetic dataset")
    p.add_argument(
        "--db",
        default="test_dbs/synthetic_offshore.db",
        help="Output database path (default: test_dbs/synthetic_offshore.db)",
    )
    p.add_argument("--reset", action="store_true", help="Delete existing DB first")
    p.add_argument("--n-entities", type=int, default=80)
    p.add_argument("--n-officers", type=int, default=50)
    p.add_argument("--seed", type=int, default=42)
    return p.parse_args()


def main() -> None:
    args = parse_args()
    counts = generate_dataset(
        db_path=args.db,
        reset=args.reset,
        n_entities=args.n_entities,
        n_officers=args.n_officers,
        seed=args.seed,
    )
    print()
    print("Counts:")
    for k, v in counts.items():
        print(f"  {k:<28} {v}")


if __name__ == "__main__":
    main()
