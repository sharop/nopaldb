"""Generador determinista de un dataset sintetico de fraude financiero.

Plantea un escenario controlado para detectar:
  - Una piramide de control corporativo (Acto 4 reusa el patron de Harbor Cay
    pero a mayor escala).
  - Un anillo de fraude (5 Persons + 5 Accounts) cuyas transferencias forman
    un ciclo casi cerrado, plantado para que community detection lo aisle.
  - Una jerarquia de clases (LegalEntity > OffshoreEntity > ShellCompany)
    via labels - pendiente OWL hasta que se exponga import_turtle a Python.

Topologia:
  Person -[OWNS]-> Account
  Account -[TRANSFERS{amount,timestamp}]-> Account
  Company -[CONTROLS{amount}]-> Company
  Person -[DIRECTS]-> Company

Determinismo: seed=42 produce siempre los mismos IDs y el mismo top-1 de
community detection del ring. El gate cruzado lo verifica.
"""

from __future__ import annotations

import argparse
import random
from pathlib import Path

import nopaldb


PERSON_FIRST = [
    "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Hugo",
    "Ines", "Javier", "Karen", "Liam", "Mei", "Nadia", "Oscar", "Petra",
    "Quentin", "Rosa", "Said", "Tomas", "Uma", "Viktor", "Walter", "Xinyi",
    "Yara", "Zane", "Anders", "Beatriz", "Carlos", "Diana",
]
PERSON_LAST = [
    "Novak", "Okafor", "Svensson", "Muller", "Petrov", "Cohen", "Tanaka",
    "Bergstrom", "Almeida", "Demir", "Andersson", "Volkov", "Ferrari",
    "Hadid", "Konstantin", "Reyes", "Park", "Yamamoto", "Schmidt", "Hassan",
]
COUNTRIES = [
    "Russia", "Nigeria", "Sweden", "Germany", "Spain", "Brazil", "Japan",
    "Iran", "Italy", "Saudi Arabia", "Turkey", "Mexico", "China", "Argentina",
]
COMPANY_PREFIXES = [
    "Global", "Northern", "Atlas", "Sunrise", "Bluewater", "Onyx", "Phoenix",
    "Helios", "Pinnacle", "Crescent", "Sapphire", "Granite",
]
COMPANY_SUFFIXES = [
    "Holdings", "Capital", "Trust", "Group", "Ventures", "Partners",
]
INDUSTRIES = ["banking", "logistics", "real estate", "tech", "energy", "trading"]


def _person_name(rng: random.Random, used: set) -> str:
    while True:
        name = f"{rng.choice(PERSON_FIRST)} {rng.choice(PERSON_LAST)}"
        if name not in used:
            used.add(name)
            return name


def _company_name(rng: random.Random) -> str:
    return f"{rng.choice(COMPANY_PREFIXES)} {rng.choice(COMPANY_SUFFIXES)}"


def generate_dataset(
    db_path: str,
    reset: bool = True,
    n_persons: int = 200,
    n_companies: int = 60,
    n_accounts: int = 300,
    n_transfers: int = 400,
    seed: int = 42,
) -> dict:
    """Construye el dataset.

    Returns:
        dict con conteos + IDs de las entidades del ring (para que el gate
        compare exactamente).
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
    print("Generating Synthetic Fraud dataset (Acto 4)")
    print("=" * 60)
    print(f"DB:           {db_path}")
    print(f"seed:         {seed}")
    print(f"n_persons:    {n_persons}")
    print(f"n_companies:  {n_companies}")
    print(f"n_accounts:   {n_accounts}")
    print(f"n_transfers:  {n_transfers}")

    counts = {
        "persons": 0,
        "companies": 0,
        "accounts": 0,
        "edges_owns": 0,
        "edges_transfers": 0,
        "edges_controls": 0,
        "edges_directs": 0,
        "ring_persons": [],
        "ring_accounts": [],
    }

    with nopaldb.Graph.open(db_path) as graph:
        # 1. Persons
        tx = graph.begin_transaction()
        person_ids = []
        used_names = set()
        for _ in range(n_persons):
            name = _person_name(rng, used_names)
            country = rng.choice(COUNTRIES)
            description = f"{name} es residente de {country} con actividad financiera regular."
            person_ids.append(
                tx.add_node(
                    "Person",
                    {"name": name, "country": country, "description": description},
                )
            )
        tx.commit()
        counts["persons"] = n_persons

        # 2. Companies (mix LegalEntity vs ShellCompany via label)
        tx = graph.begin_transaction()
        company_ids = []
        for i in range(n_companies):
            name = f"{_company_name(rng)} #{i:03d}"
            industry = rng.choice(INDUSTRIES)
            shell = rng.random() < 0.20
            label = "ShellCompany" if shell else "LegalEntity"
            description = f"{name} opera en {industry} desde su registro corporativo."
            company_ids.append(
                tx.add_node(
                    label,
                    {
                        "name": name,
                        "industry": industry,
                        "shell": shell,
                        "description": description,
                    },
                )
            )
        tx.commit()
        counts["companies"] = n_companies

        # 3. Accounts (cada Person tiene 1-2; algunas Companies tienen 1)
        tx = graph.begin_transaction()
        account_ids = []
        person_to_accounts = {}
        for pid in person_ids:
            n_acct = rng.randint(1, 2)
            person_to_accounts[pid] = []
            for _ in range(n_acct):
                aid = tx.add_node(
                    "Account",
                    {
                        "balance": round(rng.uniform(1000, 500000), 2),
                        "currency": rng.choice(["USD", "EUR", "GBP", "CHF"]),
                        "opened_at": rng.randint(1_577_836_800, 1_704_067_200),  # 2020..2024
                    },
                )
                account_ids.append(aid)
                person_to_accounts[pid].append(aid)
                tx.add_edge(pid, aid, "OWNS", {})
                counts["edges_owns"] += 1
        tx.commit()
        counts["accounts"] = len(account_ids)

        # 4. DIRECTS: cada Company tiene 1-2 Person directors
        tx = graph.begin_transaction()
        for cid in company_ids:
            n_dir = rng.randint(1, 2)
            for did in rng.sample(person_ids, k=n_dir):
                tx.add_edge(did, cid, "DIRECTS", {"role": "director"})
                counts["edges_directs"] += 1
        tx.commit()

        # 5. CONTROLS entre companies (random graph)
        tx = graph.begin_transaction()
        n_chains = max(20, n_companies // 3)
        for _ in range(n_chains):
            parent = rng.choice(company_ids)
            child = rng.choice(company_ids)
            if parent == child:
                continue
            amount = round(rng.uniform(50_000, 5_000_000), 2)
            tx.add_edge(parent, child, "CONTROLS",
                        {"amount": amount, "controlling_stake": rng.choice([True, False])})
            counts["edges_controls"] += 1
        tx.commit()

        # 6. Plantar el FRAUD RING:
        #    Tomamos las primeras 5 Persons y sus primeras 5 Accounts.
        #    Cierre ciclico de transfers de alta frecuencia/monto similar.
        ring_person_ids = person_ids[0:5]
        ring_account_ids = [person_to_accounts[pid][0] for pid in ring_person_ids]
        counts["ring_persons"] = ring_person_ids
        counts["ring_accounts"] = ring_account_ids

        # Conectar el ring DENSAMENTE para que community detection lo aisle:
        # Cada par de cuentas del ring intercambia 6 transfers (todas las
        # direcciones). Total intra-ring: 5*4 secuenciales + ~30 cruzadas = ~50.
        tx = graph.begin_transaction()
        ring_base_ts = 1_700_000_000
        ring_amounts = [950_000, 920_000, 980_000, 940_000, 960_000]
        for i in range(len(ring_account_ids)):
            src = ring_account_ids[i]
            dst = ring_account_ids[(i + 1) % len(ring_account_ids)]
            for k in range(6):
                tx.add_edge(
                    src,
                    dst,
                    "TRANSFERS",
                    {
                        "amount": ring_amounts[i],
                        "timestamp": ring_base_ts + i * 86400 + k * 3600,
                        "ring_marker": True,
                    },
                )
                counts["edges_transfers"] += 1
        # Cross transfers densas dentro del ring (todas las direcciones)
        for i, src in enumerate(ring_account_ids):
            for j, dst in enumerate(ring_account_ids):
                if i != j:
                    for k in range(2):
                        tx.add_edge(
                            src,
                            dst,
                            "TRANSFERS",
                            {
                                "amount": round(rng.uniform(20_000, 200_000), 2),
                                "timestamp": ring_base_ts + 5 * 86400 + i * 1000 + j * 100 + k,
                            },
                        )
                        counts["edges_transfers"] += 1
        tx.commit()

        # 7. Transfers fuera del ring (ruido). Excluimos las cuentas del
        # ring como sources/targets para mantener el cluster aislado.
        ring_set = set(ring_account_ids)
        non_ring_accounts = [a for a in account_ids if a not in ring_set]
        tx = graph.begin_transaction()
        target_extra = n_transfers - counts["edges_transfers"]
        added = 0
        attempts = 0
        while added < target_extra and attempts < target_extra * 4:
            attempts += 1
            src = rng.choice(non_ring_accounts)
            dst = rng.choice(non_ring_accounts)
            if src == dst:
                continue
            amount = round(rng.uniform(100, 50_000), 2)
            ts = rng.randint(1_577_836_800, 1_735_689_600)
            tx.add_edge(src, dst, "TRANSFERS", {"amount": amount, "timestamp": ts})
            counts["edges_transfers"] += 1
            added += 1
        tx.commit()

        # 8. Indices basicos
        for label, prop in [
            ("Person", "name"),
            ("LegalEntity", "name"),
            ("ShellCompany", "name"),
        ]:
            try:
                graph.execute_nql(f"create index on {label}({prop}) type hash")
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
    p = argparse.ArgumentParser(description="Generate Synthetic Fraud dataset for Acto 4")
    p.add_argument("--db", default="test_dbs/synthetic_fraud.db")
    p.add_argument("--reset", action="store_true")
    p.add_argument("--seed", type=int, default=42)
    return p.parse_args()


def main() -> None:
    args = parse_args()
    counts = generate_dataset(db_path=args.db, reset=args.reset, seed=args.seed)
    print()
    print("Counts:")
    for k, v in counts.items():
        if isinstance(v, list):
            print(f"  {k:<18} {len(v)} ids")
        else:
            print(f"  {k:<18} {v}")


if __name__ == "__main__":
    main()
