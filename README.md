# Gastos compartidos (Soroban)

Contrato inteligente en Stellar (Soroban) para llevar las cuentas de gastos compartidos entre roommates o familia: se registra quién pagó qué, el contrato calcula cuánto debe cada quien y sugiere la menor cantidad de pagos para quedar a mano.

## El problema

Cuando varias personas comparten gastos (alquiler, súper, luz, internet), casi siempre alguien paga por todos y después hay que ir cobrando. Se pierde la cuenta, hay discusiones sobre quién debe a quién y es fácil cobrar de más o de menos.

Este contrato resuelve eso con reglas simples y verificables:

- Todo gasto queda registrado en la cadena, con quién pagó y entre quiénes se divide.
- El reparto es igual entre los participantes. Si la división no es exacta, el residuo (en unidades mínimas) se reparte entre los primeros participantes, así no se pierde ni se inventa dinero.
- Cada miembro tiene un balance: positivo si le deben, negativo si debe.
- Los balances siempre suman cero.
- `settle_plan` devuelve una lista corta de pagos que deja a todos en cero.

Elegí este problema porque es cotidiano en Costa Rica (compartir casa o apartamento con amigos, o gastos familiares) y tiene lógica pequeña y clara.

## Cómo funciona

| Función | Quién la llama | Qué hace |
| --- | --- | --- |
| `init(members)` | cualquiera, una sola vez | Crea el grupo con al menos 2 miembros distintos. |
| `add_expense(payer, amount, participants)` | el que pagó (firma) | Registra un gasto y ajusta los balances. |
| `record_payment(from, to, amount)` | el que pagó (firma) | Registra un pago real hecho fuera del contrato. No puede exceder lo que `from` debe ni lo que `to` tiene a favor. |
| `balance(member)` | cualquiera | Balance de un miembro (positivo = le deben, negativo = debe). |
| `settle_plan()` | cualquiera (solo lectura) | Lista de transferencias sugeridas para quedar en cero. |
| `members()` / `expenses()` | cualquiera (solo lectura) | Miembros del grupo y registro de gastos. |

Ejemplo: Ana paga ₡9.000 de súper para Ana, Beto y Carla. Cada quien debe ₡3.000. Ana queda con +₡6.000, Beto y Carla con −₡3.000 cada uno. Si Beto paga ₡3.000 a Ana, Beto queda en 0 y Ana queda con +₡3.000. `settle_plan` sugiere justo eso: Beto y Carla pagan a Ana.

Los montos se manejan como enteros (`i128`) en la unidad mínima; el contrato no usa decimales.

## Estructura

```text
.
├── contracts
│   └── shared-expenses
│       ├── src
│       │   ├── lib.rs    # lógica del contrato
│       │   └── test.rs   # pruebas unitarias
│       └── Cargo.toml
├── Cargo.toml            # workspace
└── README.md
```

## Cómo correrlo

Requisitos: Rust con `wasm32v1-none` y la CLI de Stellar (`stellar`).

```bash
# Pruebas unitarias
cargo test

# Compilar el contrato a WASM
stellar contract build
```

El WASM queda en `target/wasm32v1-none/release/shared_expenses.wasm`.

Para desplegarlo en testnet (opcional):

```bash
stellar keys generate alice --network testnet --fund
stellar contract deploy --wasm target/wasm32v1-none/release/shared_expenses.wasm \
  --source alice --network testnet
```

## Pruebas

Las pruebas (`contracts/shared-expenses/src/test.rs`) cubren:

- **Inicialización:** no se puede inicializar dos veces, ni con un solo miembro, ni con miembros repetidos.
- **Reparto:** división igual con residuo (100 entre 3 = 34/33/33), y caso donde quien paga no participa.
- **Validaciones:** montos en cero o negativos, lista de participantes vacía, participantes repetidos, personas fuera del grupo. Ninguna llamada fallida deja cambios.
- **Pagos:** un pago reduce la deuda; no se puede pagar más de lo debido, pagar a quien no tiene saldo a favor, ni pagarse a uno mismo.
- **Liquidación:** `settle_plan` deja a todos en cero al aplicar sus pagos, usa como máximo `miembros − 1` transferencias y es de solo lectura.
- **Invariante:** los balances siempre suman cero.

Las pruebas usan `mock_all_auths`, así que verifican la lógica de negocio pero no la firma real de cada cuenta. Las restricciones de quién puede firmar (`require_auth`) están en el código, pero no tienen una prueba dedicada todavía.

## Limitaciones conocidas

- Los gastos no se pueden editar ni borrar; si hay un error se registra un gasto correctivo.
- No hay monedas ni tokens: el contrato lleva cuentas, no mueve dinero. Los pagos se hacen fuera y se registran con `record_payment`.
- El reparto es siempre igual. Divisiones por porcentaje o por consumo quedan como mejora futura.
- Quien llama `record_payment` es quien debe el dinero; no hay confirmación del que recibe.
