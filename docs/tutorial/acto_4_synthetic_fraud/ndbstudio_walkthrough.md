# Acto 4 — Walkthrough NDBStudio Web

Visualización interactiva del dataset sintético + ring fraudulento.

## Levantar

```bash
# Generar la DB primero (si no existe):
python3 tutorials/shared/synthetic_fraud_dataset.py \
  --db test_dbs/synthetic_fraud.db --reset

# Levantar NDBStudio Web:
make run-studio-web DB=test_dbs/synthetic_fraud.db
```

Abre `http://127.0.0.1:3737`.

## Paso 1 — Topología

Pega [`queries/01_topology.nql`](queries/01_topology.nql). En la vista `Schema` confirma los conteos: Account 312, Person 200, LegalEntity 50, ShellCompany 10.

## Paso 2 — Top-5 inbound (manual)

NQL en v0.4.19 no respeta `ORDER BY ... LIMIT` sobre agregaciones, así que ejecuta:

```sql
find b.id, count(*) as inbound
from (a:Account) -[:TRANSFERS]-> (b:Account)
group by b.id
```

En la vista `Table` clickea el header `inbound` para ordenar descendente. Las 5 primeras filas son el ring (cada una con 14 inbound).

## Paso 3 — Visualizar el ring

Una vez identificadas las cuentas ring por sus IDs, ejecuta:

```sql
find a.id, b.id
from (a:Account) -[:TRANSFERS]-> (b:Account)
where a.id = "<ring_id_1>" or a.id = "<ring_id_2>"
   or a.id = "<ring_id_3>" or a.id = "<ring_id_4>"
   or a.id = "<ring_id_5>"
```

(Sustituye los IDs reales del paso 2.) Cambia a vista `Graph` — verás el ciclo cerrado de transferencias.

## Lo que NDBStudio aporta

La **vista Graph** del ring cíclico es el "wow moment" del acto: estructuras circulares dense saltan a la vista, mientras que en una tabla de transfers son indistinguibles del ruido.

## Verificación

Cubierta en el [README → gates](README.md#verificación-cruzada-gates-del-acto-4). Top-5 inbound debe ser 14 transfers cada uno.
