use rust_calculator::{calculate_json, MAX_REQUEST_BYTES};
use serde_json::{json, Value};

fn request(input: Value) -> Value {
    serde_json::from_str(&calculate_json(input.to_string())).unwrap()
}

fn assert_near(value: &Value, expected: f64) {
    let actual = value.as_f64().expect("expected a JSON number");
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

fn property() -> Value {
    json!({
        "purchase_price": 2_000_000,
        "acquisition_costs": 40_000,
        "initial_capital_expenditures": 60_000,
        "annual_potential_rent": 240_000,
        "annual_other_income": 12_000,
        "vacancy_rate": 0.08,
        "annual_operating_expenses": 72_000,
        "annual_capital_expenditures": 12_000,
        "loan": {
            "principal": 1_200_000,
            "annual_interest_rate": 0.055,
            "amortization_months": 0
        }
    })
}

fn assumptions() -> Value {
    json!({
        "holding_years": 2,
        "exit_cap_rate": 0.08,
        "selling_cost_rate": 0.02,
        "discount_rate": 0.10
    })
}

#[test]
fn routes_scientific_and_finance_operations() {
    let cases = [
        (
            json!({"operation": "evaluate", "expression": "sin(30) + 2^3^2", "angle_mode": "degrees"}),
            512.5,
        ),
        (
            json!({"operation": "npv", "rate": 0.1, "cash_flows": [-100, 110]}),
            0.0,
        ),
        (json!({"operation": "irr", "cash_flows": [-100, 110]}), 0.1),
    ];
    for (input, expected) in cases {
        let response = request(input);
        assert_eq!(response["ok"], true, "{response}");
        assert_near(&response["result"]["value"], expected);
    }

    let response = request(json!({
        "operation": "loan",
        "terms": {"principal": 12_000, "annual_interest_rate": 0, "amortization_months": 24},
        "elapsed_months": 18
    }));
    assert_eq!(response["ok"], true, "{response}");
    assert_near(&response["result"]["monthly_payment"], 500.0);
    assert_near(&response["result"]["annual_debt_service"], 3_000.0);
    assert_near(&response["result"]["remaining_balance"], 3_000.0);
}

#[test]
fn underwrites_and_projects_a_property_from_json() {
    let response = request(json!({"operation": "underwrite", "property": property()}));
    assert_eq!(response["ok"], true, "{response}");
    let result = &response["result"];
    assert_near(&result["annual_noi"], 160_800.0);
    assert_near(&result["cap_rate"], 0.0804);
    assert_near(&result["debt_service_coverage_ratio"], 160_800.0 / 66_000.0);
    assert_near(&result["debt_yield"], 0.134);
    assert_near(&result["cash_on_cash_return"], 0.092);
    assert_near(&result["break_even_occupancy"], 0.575);

    let response = request(json!({
        "operation": "project",
        "property": property(),
        "assumptions": assumptions()
    }));
    assert_eq!(response["ok"], true, "{response}");
    let result = &response["result"];
    assert_near(&result["initial_equity"], 900_000.0);
    assert_near(&result["gross_sale_price"], 2_010_000.0);
    assert_near(&result["loan_payoff"], 1_200_000.0);
    assert_near(&result["years"][0]["levered_cash_flow"], 82_800.0);
    assert_near(&result["years"][1]["levered_cash_flow"], 852_600.0);
    assert_near(
        &result["levered_npv"],
        -900_000.0 + 82_800.0 / 1.10 + 852_600.0 / 1.21,
    );
}

#[test]
fn evaluates_each_sensitivity_cell_and_preserves_cell_errors() {
    let response = request(json!({
        "operation": "sensitivity",
        "property": property(),
        "assumptions": assumptions(),
        "rent_growth_rates": [0, 0.02],
        "exit_cap_rates": [0.08, 0.10, 0]
    }));
    assert_eq!(response["ok"], true, "{response}");
    let scenarios = response["result"]["scenarios"].as_array().unwrap();
    assert_eq!(scenarios.len(), 6);
    assert_near(&scenarios[0]["result"]["gross_sale_price"], 2_010_000.0);
    assert_near(&scenarios[1]["result"]["gross_sale_price"], 1_608_000.0);
    assert_eq!(scenarios[2]["ok"], false);
    assert!(scenarios[2]["error"]["message"].is_string());
    assert_near(&scenarios[3]["annual_rent_growth"], 0.02);
    assert!(scenarios[3]["result"]["gross_sale_price"].as_f64().unwrap() > 2_010_000.0);
}

#[test]
fn returns_structured_errors_for_invalid_requests() {
    for (input, code) in [
        ("{", "invalid_json"),
        (
            r#"{"operation":"evaluate","expression":"1","angle_mdoe":"degrees"}"#,
            "invalid_input",
        ),
        (
            r#"{"operation":"evaluate","expression":"sqrt(-1)"}"#,
            "domain",
        ),
        (
            r#"{"operation":"irr","cash_flows":[-100,230,-132]}"#,
            "no_solution",
        ),
        (
            r#"{"operation":"loan","terms":{"principal":1,"annual_interest_rate":0,"amortization_months":12,"baloon_months":6}}"#,
            "invalid_input",
        ),
        (&" ".repeat(MAX_REQUEST_BYTES + 1), "invalid_input"),
    ] {
        let response: Value = serde_json::from_str(&calculate_json(input)).unwrap();
        assert_eq!(response["ok"], false, "{response}");
        assert_eq!(response["error"]["code"], code, "{response}");
    }
}
