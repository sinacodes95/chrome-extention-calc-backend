//! Annual commercial property underwriting and sale projections.

use crate::finance::{irr, loan_metrics, npv, LoanTerms};
use crate::CalcError;

const MAX_HOLDING_YEARS: u32 = 50;
const MONTHS_PER_YEAR: u32 = 12;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyInputs {
    pub purchase_price: f64,
    #[serde(default)]
    pub acquisition_costs: f64,
    #[serde(default)]
    pub initial_capital_expenditures: f64,
    pub annual_potential_rent: f64,
    /// First-year other income, already adjusted for vacancy where applicable.
    #[serde(default)]
    pub annual_other_income: f64,
    pub vacancy_rate: f64,
    pub annual_operating_expenses: f64,
    #[serde(default)]
    pub annual_capital_expenditures: f64,
    pub loan: Option<LoanTerms>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Underwriting {
    pub annual_collected_rent: f64,
    pub annual_effective_gross_income: f64,
    pub annual_noi: f64,
    pub cap_rate: f64,
    pub annual_debt_service: f64,
    pub debt_service_coverage_ratio: Option<f64>,
    pub debt_yield: Option<f64>,
    pub loan_to_value: f64,
    pub initial_equity: f64,
    pub annual_cash_flow_before_debt: f64,
    pub annual_cash_flow_after_debt: f64,
    pub cash_on_cash_return: Option<f64>,
    /// Required occupancy after operating costs, capital expenditures and debt service.
    /// A value above 1 indicates that rent cannot cover the costs even at full occupancy.
    pub break_even_occupancy: Option<f64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionAssumptions {
    pub holding_years: u32,
    #[serde(default)]
    pub annual_rent_growth: f64,
    #[serde(default)]
    pub annual_other_income_growth: f64,
    #[serde(default)]
    pub annual_expense_growth: f64,
    #[serde(default)]
    pub annual_capex_growth: f64,
    pub exit_cap_rate: f64,
    #[serde(default)]
    pub selling_cost_rate: f64,
    pub discount_rate: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProjectionYear {
    pub year: u32,
    pub annual_collected_rent: f64,
    pub annual_other_income: f64,
    pub annual_effective_gross_income: f64,
    pub annual_operating_expenses: f64,
    pub annual_noi: f64,
    pub annual_capital_expenditures: f64,
    pub annual_debt_service: f64,
    /// Balance after scheduled payments and before any sale payoff.
    pub ending_loan_balance: f64,
    /// Includes net sale proceeds in the last year.
    pub unlevered_cash_flow: f64,
    /// Includes net sale proceeds and any loan payoff in the last year.
    pub levered_cash_flow: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Projection {
    pub initial_unlevered_investment: f64,
    pub initial_equity: f64,
    pub forward_noi: f64,
    pub gross_sale_price: f64,
    pub selling_costs: f64,
    /// Sale price less selling costs, before any loan payoff.
    pub net_sale_proceeds: f64,
    pub loan_payoff: f64,
    pub unlevered_npv: f64,
    pub levered_npv: f64,
    /// An IRR is available for cash flows with one sign change and a representable root.
    pub unlevered_irr: Option<f64>,
    /// An IRR is available for cash flows with one sign change and a representable root.
    pub levered_irr: Option<f64>,
    /// Positive net equity cash flows divided by the absolute sum of negative ones.
    pub equity_multiple: Option<f64>,
    pub years: Vec<ProjectionYear>,
}

#[derive(Clone, Copy)]
struct Operations {
    collected_rent: f64,
    other_income: f64,
    operating_expenses: f64,
    capital_expenditures: f64,
}

impl Operations {
    fn first_year(inputs: &PropertyInputs) -> Self {
        Self {
            collected_rent: inputs.annual_potential_rent * (1.0 - inputs.vacancy_rate),
            other_income: inputs.annual_other_income,
            operating_expenses: inputs.annual_operating_expenses,
            capital_expenditures: inputs.annual_capital_expenditures,
        }
    }

    fn next_year(self, assumptions: &ProjectionAssumptions) -> Self {
        Self {
            collected_rent: self.collected_rent * (1.0 + assumptions.annual_rent_growth),
            other_income: self.other_income * (1.0 + assumptions.annual_other_income_growth),
            operating_expenses: self.operating_expenses * (1.0 + assumptions.annual_expense_growth),
            capital_expenditures: self.capital_expenditures
                * (1.0 + assumptions.annual_capex_growth),
        }
    }

    fn effective_gross_income(self) -> f64 {
        self.collected_rent + self.other_income
    }

    fn noi(self) -> f64 {
        self.effective_gross_income() - self.operating_expenses
    }

    fn unlevered_cash_flow(self) -> f64 {
        self.noi() - self.capital_expenditures
    }
}

impl PropertyInputs {
    fn initial_investment(&self) -> Result<(f64, f64), CalcError> {
        if !self.purchase_price.is_finite() || self.purchase_price <= 0.0 {
            return Err(CalcError::InvalidInput(
                "purchase_price must be finite and greater than zero".into(),
            ));
        }
        for (name, amount) in [
            ("acquisition_costs", self.acquisition_costs),
            (
                "initial_capital_expenditures",
                self.initial_capital_expenditures,
            ),
            ("annual_potential_rent", self.annual_potential_rent),
            ("annual_other_income", self.annual_other_income),
            ("annual_operating_expenses", self.annual_operating_expenses),
            (
                "annual_capital_expenditures",
                self.annual_capital_expenditures,
            ),
        ] {
            if !amount.is_finite() || amount < 0.0 {
                return Err(CalcError::InvalidInput(format!(
                    "{name} must be finite and nonnegative"
                )));
            }
        }
        validate_fraction("vacancy_rate", self.vacancy_rate)?;

        let total =
            self.purchase_price + self.acquisition_costs + self.initial_capital_expenditures;
        ensure_finite(total, "initial investment")?;
        let equity = total - self.loan_principal();
        if equity < 0.0 {
            return Err(CalcError::InvalidInput(
                "loan principal must not exceed the total initial investment".into(),
            ));
        }
        Ok((total, equity))
    }

    fn loan_principal(&self) -> f64 {
        self.loan.as_ref().map_or(0.0, |loan| loan.principal)
    }

    fn debt(&self, elapsed_months: u32) -> Result<(f64, f64), CalcError> {
        match &self.loan {
            Some(loan) => {
                let metrics = loan_metrics(loan, elapsed_months)?;
                Ok((metrics.annual_debt_service, metrics.remaining_balance))
            }
            None => Ok((0.0, 0.0)),
        }
    }
}

impl ProjectionAssumptions {
    fn validate(&self) -> Result<(), CalcError> {
        if !(1..=MAX_HOLDING_YEARS).contains(&self.holding_years) {
            return Err(CalcError::InvalidInput(format!(
                "holding_years must be between 1 and {MAX_HOLDING_YEARS}"
            )));
        }
        for (name, rate) in [
            ("annual_rent_growth", self.annual_rent_growth),
            (
                "annual_other_income_growth",
                self.annual_other_income_growth,
            ),
            ("annual_expense_growth", self.annual_expense_growth),
            ("annual_capex_growth", self.annual_capex_growth),
        ] {
            if !rate.is_finite() || rate < -1.0 {
                return Err(CalcError::InvalidInput(format!(
                    "{name} must be finite and at least -1"
                )));
            }
        }
        if !self.exit_cap_rate.is_finite() || self.exit_cap_rate <= 0.0 {
            return Err(CalcError::InvalidInput(
                "exit_cap_rate must be finite and greater than zero".into(),
            ));
        }
        validate_fraction("selling_cost_rate", self.selling_cost_rate)?;
        if !self.discount_rate.is_finite() || self.discount_rate <= -1.0 {
            return Err(CalcError::InvalidInput(
                "discount_rate must be finite and greater than -1".into(),
            ));
        }
        Ok(())
    }
}

fn validate_fraction(name: &str, value: f64) -> Result<(), CalcError> {
    if !(0.0..=1.0).contains(&value) {
        return Err(CalcError::InvalidInput(format!(
            "{name} must be between 0 and 1"
        )));
    }
    Ok(())
}

fn ensure_finite(value: f64, metric: &str) -> Result<f64, CalcError> {
    if !value.is_finite() {
        return Err(CalcError::Numerical(format!(
            "{metric} exceeds the supported numeric range"
        )));
    }
    Ok(value)
}

fn ratio(numerator: f64, denominator: f64, metric: &str) -> Result<Option<f64>, CalcError> {
    if denominator == 0.0 {
        return Ok(None);
    }
    ensure_finite(numerator / denominator, metric).map(Some)
}

/// Computes first-year operating and debt metrics with annual inputs and decimal rates.
pub fn underwrite(inputs: &PropertyInputs) -> Result<Underwriting, CalcError> {
    let (_, initial_equity) = inputs.initial_investment()?;
    let (annual_debt_service, _) = inputs.debt(0)?;
    let operations = Operations::first_year(inputs);
    let annual_noi = ensure_finite(operations.noi(), "net operating income")?;
    let annual_cash_flow_before_debt =
        ensure_finite(operations.unlevered_cash_flow(), "cash flow before debt")?;
    let annual_cash_flow_after_debt = ensure_finite(
        annual_cash_flow_before_debt - annual_debt_service,
        "cash flow after debt",
    )?;
    let uncovered_costs =
        operations.operating_expenses + operations.capital_expenditures + annual_debt_service
            - operations.other_income;
    let loan_principal = inputs.loan_principal();
    Ok(Underwriting {
        annual_collected_rent: operations.collected_rent,
        annual_effective_gross_income: operations.effective_gross_income(),
        annual_noi,
        cap_rate: ensure_finite(annual_noi / inputs.purchase_price, "cap rate")?,
        annual_debt_service,
        debt_service_coverage_ratio: ratio(
            annual_noi,
            annual_debt_service,
            "debt service coverage ratio",
        )?,
        debt_yield: ratio(annual_noi, loan_principal, "debt yield")?,
        loan_to_value: ensure_finite(loan_principal / inputs.purchase_price, "loan to value")?,
        initial_equity,
        annual_cash_flow_before_debt,
        annual_cash_flow_after_debt,
        cash_on_cash_return: ratio(annual_cash_flow_after_debt, initial_equity, "cash on cash")?,
        break_even_occupancy: ratio(
            uncovered_costs.max(0.0),
            inputs.annual_potential_rent,
            "break-even occupancy",
        )?,
    })
}

/// Projects annual pre-tax cash flows and a sale at the end of the holding period.
pub fn project(
    inputs: &PropertyInputs,
    assumptions: &ProjectionAssumptions,
) -> Result<Projection, CalcError> {
    assumptions.validate()?;
    let (initial_unlevered_investment, initial_equity) = inputs.initial_investment()?;
    let (mut annual_debt_service, _) = inputs.debt(0)?;
    let mut operations = Operations::first_year(inputs);
    let mut years = Vec::with_capacity(assumptions.holding_years as usize);
    for year in 1..=assumptions.holding_years {
        let (next_debt_service, ending_loan_balance) = inputs.debt(year * MONTHS_PER_YEAR)?;
        let unlevered_cash_flow =
            ensure_finite(operations.unlevered_cash_flow(), "unlevered cash flow")?;
        let levered_cash_flow = ensure_finite(
            unlevered_cash_flow - annual_debt_service,
            "levered cash flow",
        )?;
        years.push(ProjectionYear {
            year,
            annual_collected_rent: operations.collected_rent,
            annual_other_income: operations.other_income,
            annual_effective_gross_income: operations.effective_gross_income(),
            annual_operating_expenses: operations.operating_expenses,
            annual_noi: operations.noi(),
            annual_capital_expenditures: operations.capital_expenditures,
            annual_debt_service,
            ending_loan_balance,
            unlevered_cash_flow,
            levered_cash_flow,
        });
        operations = operations.next_year(assumptions);
        annual_debt_service = next_debt_service;
    }

    let forward_noi = ensure_finite(operations.noi(), "forward net operating income")?;
    if forward_noi < 0.0 {
        return Err(CalcError::Domain(
            "negative forward NOI cannot be valued using an exit cap rate".into(),
        ));
    }
    let gross_sale_price =
        ensure_finite(forward_noi / assumptions.exit_cap_rate, "gross sale price")?;
    let selling_costs = gross_sale_price * assumptions.selling_cost_rate;
    let net_sale_proceeds = gross_sale_price - selling_costs;
    let exit_year = &mut years[(assumptions.holding_years - 1) as usize];
    let loan_payoff = exit_year.ending_loan_balance;
    exit_year.unlevered_cash_flow = ensure_finite(
        exit_year.unlevered_cash_flow + net_sale_proceeds,
        "unlevered cash flow at sale",
    )?;
    exit_year.levered_cash_flow = ensure_finite(
        exit_year.levered_cash_flow + (net_sale_proceeds - loan_payoff),
        "levered cash flow at sale",
    )?;

    let unlevered_flows: Vec<_> = std::iter::once(-initial_unlevered_investment)
        .chain(years.iter().map(|year| year.unlevered_cash_flow))
        .collect();
    let levered_flows: Vec<_> = std::iter::once(-initial_equity)
        .chain(years.iter().map(|year| year.levered_cash_flow))
        .collect();
    let distributions = levered_flows
        .iter()
        .copied()
        .filter(|&flow| flow > 0.0)
        .sum();
    let contributions = -levered_flows
        .iter()
        .copied()
        .filter(|&flow| flow < 0.0)
        .sum::<f64>();
    let equity_multiple = ratio(
        ensure_finite(distributions, "total equity distributions")?,
        ensure_finite(contributions, "total equity contributions")?,
        "equity multiple",
    )?;

    Ok(Projection {
        initial_unlevered_investment,
        initial_equity,
        forward_noi,
        gross_sale_price,
        selling_costs,
        net_sale_proceeds,
        loan_payoff,
        unlevered_npv: npv(assumptions.discount_rate, &unlevered_flows)?,
        levered_npv: npv(assumptions.discount_rate, &levered_flows)?,
        unlevered_irr: optional_irr(&unlevered_flows)?,
        levered_irr: optional_irr(&levered_flows)?,
        equity_multiple,
        years,
    })
}

fn optional_irr(cash_flows: &[f64]) -> Result<Option<f64>, CalcError> {
    match irr(cash_flows) {
        Ok(rate) => Ok(Some(rate)),
        Err(CalcError::NoSolution(_) | CalcError::Numerical(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property() -> PropertyInputs {
        PropertyInputs {
            purchase_price: 1_000_000.0,
            acquisition_costs: 30_000.0,
            initial_capital_expenditures: 20_000.0,
            annual_potential_rent: 130_000.0,
            annual_other_income: 8_000.0,
            vacancy_rate: 0.1,
            annual_operating_expenses: 45_000.0,
            annual_capital_expenditures: 5_000.0,
            loan: Some(LoanTerms {
                principal: 600_000.0,
                annual_interest_rate: 0.05,
                amortization_months: 0,
                interest_only_months: 0,
            }),
        }
    }

    fn assumptions(holding_years: u32) -> ProjectionAssumptions {
        ProjectionAssumptions {
            holding_years,
            annual_rent_growth: 0.0,
            annual_other_income_growth: 0.0,
            annual_expense_growth: 0.0,
            annual_capex_growth: 0.0,
            exit_cap_rate: 0.08,
            selling_cost_rate: 0.02,
            discount_rate: 0.1,
        }
    }

    fn close(actual: f64, expected: f64) {
        let tolerance = 1e-9 * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() < tolerance,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn underwrites_vacancy_noi_debt_and_cash_occupancy() {
        let result = underwrite(&property()).unwrap();
        close(result.annual_collected_rent, 117_000.0);
        close(result.annual_effective_gross_income, 125_000.0);
        close(result.annual_noi, 80_000.0);
        close(result.cap_rate, 0.08);
        close(result.annual_debt_service, 30_000.0);
        close(result.debt_service_coverage_ratio.unwrap(), 80.0 / 30.0);
        close(result.debt_yield.unwrap(), 80.0 / 600.0);
        close(result.loan_to_value, 0.6);
        close(result.initial_equity, 450_000.0);
        close(result.annual_cash_flow_before_debt, 75_000.0);
        close(result.annual_cash_flow_after_debt, 45_000.0);
        close(result.cash_on_cash_return.unwrap(), 0.1);
        close(result.break_even_occupancy.unwrap(), 72.0 / 130.0);
    }

    #[test]
    fn break_even_occupancy_is_zero_when_other_income_covers_costs() {
        let mut inputs = property();
        inputs.annual_other_income = 100_000.0;
        assert_eq!(underwrite(&inputs).unwrap().break_even_occupancy, Some(0.0));
    }

    #[test]
    fn sale_uses_forward_noi_and_pays_off_debt_after_operating_cash_flow() {
        let mut exit = assumptions(1);
        exit.annual_rent_growth = 0.04;
        exit.annual_other_income_growth = 0.02;
        exit.annual_expense_growth = 0.03;
        let result = project(&property(), &exit).unwrap();
        let forward_noi = 117_000.0 * 1.04 + 8_000.0 * 1.02 - 45_000.0 * 1.03;
        let net_sale = (forward_noi / 0.08) * 0.98;
        let final_equity_cash_flow = 45_000.0 + net_sale - 600_000.0;
        close(result.forward_noi, forward_noi);
        close(result.net_sale_proceeds, net_sale);
        close(result.loan_payoff, 600_000.0);
        close(result.years[0].annual_noi, 80_000.0);
        close(result.years[0].unlevered_cash_flow, 75_000.0 + net_sale);
        close(result.years[0].levered_cash_flow, final_equity_cash_flow);
        close(
            result.levered_npv,
            -450_000.0 + final_equity_cash_flow / 1.1,
        );
        close(
            result.levered_irr.unwrap(),
            final_equity_cash_flow / 450_000.0 - 1.0,
        );
        close(
            result.equity_multiple.unwrap(),
            final_equity_cash_flow / 450_000.0,
        );
    }

    #[test]
    fn tracks_partial_year_debt_service_and_maturity() {
        let mut inputs = property();
        inputs.loan = Some(LoanTerms {
            principal: 60_000.0,
            annual_interest_rate: 0.0,
            amortization_months: 15,
            interest_only_months: 0,
        });
        let result = project(&inputs, &assumptions(3)).unwrap();
        close(result.years[0].annual_debt_service, 48_000.0);
        close(result.years[0].ending_loan_balance, 12_000.0);
        close(result.years[1].annual_debt_service, 12_000.0);
        close(result.years[2].annual_debt_service, 0.0);
        close(result.loan_payoff, 0.0);
    }

    #[test]
    fn returns_defined_outputs_without_debt_rent_or_an_irr() {
        let mut inputs = property();
        inputs.loan = None;
        inputs.annual_potential_rent = 0.0;
        inputs.annual_operating_expenses = inputs.annual_other_income;
        let underwriting = underwrite(&inputs).unwrap();
        assert_eq!(underwriting.debt_service_coverage_ratio, None);
        assert_eq!(underwriting.debt_yield, None);
        assert_eq!(underwriting.break_even_occupancy, None);
        let result = project(&inputs, &assumptions(2)).unwrap();
        assert_eq!(result.levered_irr, None);
        assert_eq!(result.unlevered_irr, None);
        assert_eq!(result.levered_npv, result.unlevered_npv);
        assert_eq!(result.gross_sale_price, 0.0);
    }

    #[test]
    fn rejects_invalid_vacancy_and_negative_forward_noi() {
        let mut inputs = property();
        inputs.vacancy_rate = f64::NAN;
        assert!(matches!(
            underwrite(&inputs),
            Err(CalcError::InvalidInput(_))
        ));
        inputs.vacancy_rate = 1.0;
        assert!(matches!(
            project(&inputs, &assumptions(1)),
            Err(CalcError::Domain(_))
        ));
    }
}
