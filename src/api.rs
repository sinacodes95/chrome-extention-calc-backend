use crate::finance::{self, LoanMetrics, LoanTerms};
use crate::property::{self, Projection, ProjectionAssumptions, PropertyInputs, Underwriting};
use crate::scientific::{self, AngleMode};
use crate::CalcError;
use serde::{Deserialize, Serialize};

pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_AXIS_SIZE: usize = 11;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Evaluate {
        expression: String,
        angle_mode: Option<AngleMode>,
    },
    Loan {
        terms: LoanTerms,
        #[serde(default)]
        elapsed_months: u32,
    },
    Npv {
        rate: f64,
        cash_flows: Vec<f64>,
    },
    Irr {
        cash_flows: Vec<f64>,
    },
    Underwrite {
        property: PropertyInputs,
    },
    Project {
        property: PropertyInputs,
        assumptions: ProjectionAssumptions,
    },
    Sensitivity {
        property: PropertyInputs,
        assumptions: ProjectionAssumptions,
        rent_growth_rates: Vec<f64>,
        exit_cap_rates: Vec<f64>,
    },
}

#[derive(Serialize)]
#[serde(untagged)]
enum Calculation {
    Scalar { value: f64 },
    Loan(LoanMetrics),
    Underwriting(Underwriting),
    Projection(Projection),
    Sensitivity { scenarios: Vec<Scenario> },
}

#[derive(Serialize)]
struct Scenario {
    annual_rent_growth: f64,
    exit_cap_rate: f64,
    #[serde(flatten)]
    outcome: Response<ScenarioMetrics>,
}

#[derive(Serialize)]
struct ScenarioMetrics {
    gross_sale_price: f64,
    unlevered_npv: f64,
    levered_npv: f64,
    unlevered_irr: Option<f64>,
    levered_irr: Option<f64>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Response<T> {
    Success { ok: bool, result: T },
    Failure { ok: bool, error: ErrorDetail },
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl From<CalcError> for ErrorDetail {
    fn from(error: CalcError) -> Self {
        let code = match error {
            CalcError::InvalidInput(_) => "invalid_input",
            CalcError::Domain(_) => "domain",
            CalcError::Numerical(_) => "numerical",
            CalcError::NoSolution(_) => "no_solution",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl<T> From<Result<T, CalcError>> for Response<T> {
    fn from(result: Result<T, CalcError>) -> Self {
        match result {
            Ok(result) => Self::Success { ok: true, result },
            Err(error) => Self::Failure {
                ok: false,
                error: error.into(),
            },
        }
    }
}

/// Process one JSON request and return either a result or a structured error.
pub fn calculate_json(request: impl AsRef<[u8]>) -> String {
    let request = request.as_ref();
    let response = if request.len() > MAX_REQUEST_BYTES {
        Response::from(Err(CalcError::InvalidInput(format!(
            "request exceeds {MAX_REQUEST_BYTES} bytes"
        ))))
    } else {
        match serde_json::from_slice(request) {
            Ok(request) => Response::from(calculate(request)),
            Err(error) => Response::Failure {
                ok: false,
                error: ErrorDetail {
                    code: if error.is_data() {
                        "invalid_input"
                    } else {
                        "invalid_json"
                    },
                    message: error.to_string(),
                },
            },
        }
    };

    serde_json::to_string(&response).unwrap_or_else(|_| {
        r#"{"ok":false,"error":{"code":"numerical","message":"could not encode result"}}"#.into()
    })
}

fn calculate(request: Request) -> Result<Calculation, CalcError> {
    match request {
        Request::Evaluate {
            expression,
            angle_mode,
        } => Ok(Calculation::Scalar {
            value: scientific::evaluate(&expression, angle_mode.unwrap_or(AngleMode::Radians))?,
        }),
        Request::Loan {
            terms,
            elapsed_months,
        } => Ok(Calculation::Loan(finance::loan_metrics(
            &terms,
            elapsed_months,
        )?)),
        Request::Npv { rate, cash_flows } => Ok(Calculation::Scalar {
            value: finance::npv(rate, &cash_flows)?,
        }),
        Request::Irr { cash_flows } => Ok(Calculation::Scalar {
            value: finance::irr(&cash_flows)?,
        }),
        Request::Underwrite { property } => {
            Ok(Calculation::Underwriting(property::underwrite(&property)?))
        }
        Request::Project {
            property,
            assumptions,
        } => Ok(Calculation::Projection(property::project(
            &property,
            &assumptions,
        )?)),
        Request::Sensitivity {
            property,
            assumptions,
            rent_growth_rates,
            exit_cap_rates,
        } => Ok(Calculation::Sensitivity {
            scenarios: sensitivity(&property, assumptions, &rent_growth_rates, &exit_cap_rates)?,
        }),
    }
}

fn sensitivity(
    property: &PropertyInputs,
    mut assumptions: ProjectionAssumptions,
    rent_growth_rates: &[f64],
    exit_cap_rates: &[f64],
) -> Result<Vec<Scenario>, CalcError> {
    if !(1..=MAX_AXIS_SIZE).contains(&rent_growth_rates.len())
        || !(1..=MAX_AXIS_SIZE).contains(&exit_cap_rates.len())
    {
        return Err(CalcError::InvalidInput(format!(
            "each sensitivity axis needs between 1 and {MAX_AXIS_SIZE} rates"
        )));
    }
    property::underwrite(property)?;

    let mut scenarios = Vec::with_capacity(rent_growth_rates.len() * exit_cap_rates.len());
    for &annual_rent_growth in rent_growth_rates {
        assumptions.annual_rent_growth = annual_rent_growth;
        for &exit_cap_rate in exit_cap_rates {
            assumptions.exit_cap_rate = exit_cap_rate;
            let result =
                property::project(property, &assumptions).map(|projection| ScenarioMetrics {
                    gross_sale_price: projection.gross_sale_price,
                    unlevered_npv: projection.unlevered_npv,
                    levered_npv: projection.levered_npv,
                    unlevered_irr: projection.unlevered_irr,
                    levered_irr: projection.levered_irr,
                });
            scenarios.push(Scenario {
                annual_rent_growth,
                exit_cap_rate,
                outcome: result.into(),
            });
        }
    }
    Ok(scenarios)
}
