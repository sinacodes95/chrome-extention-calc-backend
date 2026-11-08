# Rust Calculator

A scientific calculator and commercial property investment engine with a JSON interface for the command line and WebAssembly.

## Run locally

Install a stable Rust toolchain, then run these commands from this directory.

```sh
cargo test --locked
printf '%s\n' '{"operation":"evaluate","expression":"sin(30) + log(64, 2) + 5!","angle_mode":"degrees"}' | cargo run --locked --quiet
```

```json
{"ok":true,"result":{"value":126.5}}
```

The CLI reads one UTF-8 JSON request from standard input and writes one JSON response. Rust callers can use `rust_calculator::calculate_json` or the typed functions in the `scientific`, `finance`, and `property` modules.

## Build WebAssembly

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked --release --lib --target wasm32-unknown-unknown
```

Package `target/wasm32-unknown-unknown/release/rust_calculator.wasm` and `web/calculator-wasm.js` together. The loader accepts the module bytes and returns a synchronous function using the same JSON requests and responses as the CLI. In a browser module with both files next to it:

```js
import { loadCalculator } from './calculator-wasm.js';

const response = await fetch(new URL('./rust_calculator.wasm', import.meta.url));
const calculateJson = await loadCalculator(await response.arrayBuffer());
const request = { operation: 'evaluate', expression: '2^3' };
console.log(JSON.parse(calculateJson(JSON.stringify(request))));
```

The module runs locally and does not need a service. See the sibling `chrome-extention-property-calc-frontend` repository for the Chrome extension build and installation steps.

## JSON API

| Operation | Request fields besides `operation` | Result |
| --- | --- | --- |
| `evaluate` | `expression`, optional `angle_mode` of `radians` (default) or `degrees` | `value` |
| `npv` | `rate`, `cash_flows` | `value` in the input currency |
| `irr` | `cash_flows` | `value` as a decimal rate per period |
| `loan` | `terms`, optional `elapsed_months` (defaults to 0) | Next monthly payment, scheduled debt service over the next 12 months, current balance, and principal and interest paid to date |
| `underwrite` | `property` | Year-one rent, effective income, NOI, cap rate, debt service, DSCR, debt yield, LTV, equity, cash flows, cash-on-cash return, and break-even occupancy |
| `project` | `property`, `assumptions` | Annual cash flows, sale valuation, payoff, and levered and unlevered NPV and IRR, plus equity multiple |
| `sensitivity` | `property`, `assumptions`, `rent_growth_rates`, `exit_cap_rates` | `scenarios` with sale price, NPV and IRR for each combination |

Use decimal rates such as `0.08` for 8% and one consistent currency for all amounts. Inputs and outputs use floating-point numbers. Round for display in the caller.

Successful requests return `{"ok":true,"result":{...}}`. Errors return `{"ok":false,"error":{"code":"domain","message":"..."}}`. Error codes are `invalid_json`, `invalid_input`, `domain`, `numerical`, and `no_solution`. Requests are limited to 64 KiB. Undefined property ratios and unavailable projected IRRs appear as `null`.

### Scientific expressions

Use `+`, `-`, `*`, `/`, signed remainder `%`, power `^`, factorial `!`, and parentheses. Multiplication requires `*`. Exponentiation groups from the right, so `2^3^2` is 512, and `-2^2` is −4. Factorial accepts integers from 0 to 170. Constants `pi`, `tau`, and `e` and function names are case-insensitive. Scientific notation such as `1.2e-3` is supported.

| Category | Functions |
| --- | --- |
| Trigonometry | `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2(y, x)` |
| Hyperbolic | `sinh`, `cosh`, `tanh` |
| Powers and roots | `sqrt`, `cbrt`, `exp`, `pow(x, y)`, `hypot(x, y)` |
| Logarithms | `ln`, `log2`, `log10`, `log(x)` (base 10), `log(x, base)` |
| Rounding and comparison | `abs`, `floor`, `ceil`, `round`, `trunc`, `min(x, y)`, `max(x, y)` |

Angle mode controls trigonometric inputs and inverse trigonometric outputs. Expressions are limited to 4,096 bytes and 64 levels of parsing depth.

### Property projection example

Save this request as `deal.json` and run `cargo run --locked --quiet < deal.json`.

```json
{
  "operation": "project",
  "property": {
    "purchase_price": 2000000,
    "acquisition_costs": 40000,
    "initial_capital_expenditures": 60000,
    "annual_potential_rent": 240000,
    "annual_other_income": 12000,
    "vacancy_rate": 0.08,
    "annual_operating_expenses": 72000,
    "annual_capital_expenditures": 12000,
    "loan": {
      "principal": 1200000,
      "annual_interest_rate": 0.055,
      "amortization_months": 300
    }
  },
  "assumptions": {
    "holding_years": 5,
    "annual_rent_growth": 0.03,
    "annual_other_income_growth": 0.02,
    "annual_expense_growth": 0.025,
    "annual_capex_growth": 0.025,
    "exit_cap_rate": 0.085,
    "selling_cost_rate": 0.02,
    "discount_rate": 0.10
  }
}
```

This example yields year-one NOI of 160,800, a gross sale price of about 2,208,886, and an 11.13% levered IRR.

- Year one uses the stated operating inputs. Growth compounds from year two. Vacancy applies to potential rent at a constant rate, and other income is entered after any losses. NOI equals effective gross income less operating expenses. Recurring capital expenditures reduce cash flow below NOI. Cap rate is year-one NOI divided by purchase price, DSCR is NOI divided by scheduled debt service, and debt yield is NOI divided by original debt. [OCC Commercial Real Estate Lending Handbook](https://www.occ.gov/publications-and-resources/publications/comptrollers-handbook/files/commercial-real-estate-lending/pub-ch-commercial-real-estate.pdf)
- Cash-on-cash return uses cash flow after annual capital expenditures and debt service divided by initial equity. Break-even occupancy is the share of potential rent needed to cover operating expenses, recurring capital expenditures, and debt service after other income. It is zero if other income covers those costs and can exceed 100% when full occupancy would still leave a shortfall.
- The projection supports 1 to 50 years, with initial investment at time zero and year-end cash flows. Gross sale price is the following year's NOI divided by exit cap rate. The final cash flows include sale proceeds after selling costs, and the levered cash flow also subtracts the remaining loan balance. Both NPVs use the supplied discount rate. Cash flows are pre-tax and use a single fixed-rate loan through sale.
- Monthly loan interest uses the annual nominal rate divided by 12. Optional `interest_only_months` precedes the amortization schedule. Set `amortization_months` to 0 for a fully interest-only loan with the principal due at sale in the projection. Leave `loan` out for an all-cash purchase.
- Standalone NPV and IRR treat the first entry as time zero and each later entry as one period apart. IRR requires one sign change across nonzero cash flows. Projected IRR is `null` when a unique rate is unavailable under that rule.
- Acquisition costs, initial and annual capital expenditures, other income, growth rates, and selling cost rate default to zero if omitted.

To run a sensitivity grid, change `operation` to `sensitivity` and add arrays such as `"rent_growth_rates":[0,0.03,0.05]` and `"exit_cap_rates":[0.075,0.085,0.095]`. Each axis accepts 1 to 11 rates. The grid replaces the corresponding rates in `assumptions` and retains its other inputs. Results follow rent-growth order and then exit-cap order. Each scenario has its own `ok`, with either `result` or `error`.
