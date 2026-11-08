//! Periodic investment returns and fixed-rate loan calculations.

use crate::CalcError;

/// A fixed-rate loan with monthly payments. Interest rates are decimals.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoanTerms {
    pub principal: f64,
    pub annual_interest_rate: f64,
    /// Number of amortizing payments following the interest-only period.
    /// Zero means interest-only until an external payoff.
    pub amortization_months: u32,
    /// Scheduled interest-only payments before amortization starts.
    /// Use zero here for an entirely interest-only loan.
    #[serde(default)]
    pub interest_only_months: u32,
}

/// Scheduled loan amounts after a given number of monthly payments.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct LoanMetrics {
    /// The payment due next month.
    pub monthly_payment: f64,
    /// Total scheduled payments due over the next twelve months.
    pub annual_debt_service: f64,
    pub remaining_balance: f64,
    pub principal_paid: f64,
    pub interest_paid: f64,
}

/// Calculate balances and payments for a fixed-rate loan. Past the end of
/// amortization, the balance and future scheduled payments are zero.
pub fn loan_metrics(terms: &LoanTerms, elapsed_months: u32) -> Result<LoanMetrics, CalcError> {
    if !terms.principal.is_finite() || terms.principal < 0.0 {
        return Err(CalcError::InvalidInput(
            "loan principal must be finite and nonnegative".into(),
        ));
    }
    if !terms.annual_interest_rate.is_finite() || terms.annual_interest_rate < 0.0 {
        return Err(CalcError::InvalidInput(
            "annual loan interest rate must be finite and nonnegative".into(),
        ));
    }
    if terms.amortization_months == 0 && terms.interest_only_months != 0 {
        return Err(CalcError::InvalidInput(
            "use zero interest-only months when amortization months is zero".into(),
        ));
    }

    let elapsed = u64::from(elapsed_months);
    let amortization = u64::from(terms.amortization_months);
    let interest_only = if amortization == 0 {
        u64::MAX
    } else {
        u64::from(terms.interest_only_months)
    };
    let interest_only_paid = elapsed.min(interest_only);
    let amortizing_paid = elapsed.saturating_sub(interest_only).min(amortization);
    let amortizing_left = amortization - amortizing_paid;

    let monthly_rate = terms.annual_interest_rate / 12.0;
    let interest_only_payment = terms.principal * monthly_rate;
    let (amortizing_payment, principal_paid, remaining_balance) = if amortization == 0 {
        (0.0, 0.0, terms.principal)
    } else if monthly_rate == 0.0 {
        (
            terms.principal / amortization as f64,
            terms.principal * (amortizing_paid as f64 / amortization as f64),
            terms.principal * (amortizing_left as f64 / amortization as f64),
        )
    } else {
        let log_growth = monthly_rate.ln_1p();
        let discount_left = (-log_growth * amortizing_left as f64).exp();
        let annuity_denominator = -(-log_growth * amortization as f64).exp_m1();
        let principal_factor = -(-log_growth * amortizing_paid as f64).exp_m1();
        let remaining_factor = -(-log_growth * amortizing_left as f64).exp_m1();
        (
            terms.principal * (monthly_rate / annuity_denominator),
            terms.principal * (discount_left * (principal_factor / annuity_denominator)),
            terms.principal * (remaining_factor / annuity_denominator),
        )
    };

    let monthly_payment = if elapsed < interest_only {
        interest_only_payment
    } else if amortizing_left > 0 {
        amortizing_payment
    } else {
        0.0
    };
    let interest_only_next_year = interest_only.saturating_sub(elapsed).min(12);
    let amortizing_next_year = amortizing_left.min(12 - interest_only_next_year);
    let annual_debt_service = (interest_only_next_year as f64).mul_add(
        interest_only_payment,
        amortizing_next_year as f64 * amortizing_payment,
    );
    let interest_paid = if monthly_rate == 0.0 {
        0.0
    } else {
        (interest_only_paid as f64).mul_add(
            interest_only_payment,
            (amortizing_paid as f64).mul_add(amortizing_payment, -principal_paid),
        )
    };

    if [
        monthly_payment,
        annual_debt_service,
        interest_paid,
        remaining_balance,
    ]
    .iter()
    .any(|amount| !amount.is_finite())
    {
        return Err(CalcError::Numerical(
            "loan amounts exceed the numeric range".into(),
        ));
    }

    Ok(LoanMetrics {
        monthly_payment,
        annual_debt_service,
        remaining_balance,
        principal_paid,
        // Guard against rounding in the difference between payments and principal.
        interest_paid: interest_paid.max(0.0),
    })
}

/// Net present value with a per-period decimal rate and `cash_flows[0]`
/// occurring immediately. Later entries occur at the ends of equal periods.
pub fn npv(rate: f64, cash_flows: &[f64]) -> Result<f64, CalcError> {
    if !rate.is_finite() || rate <= -1.0 {
        return Err(CalcError::InvalidInput(
            "discount rate must be finite and greater than -1".into(),
        ));
    }
    validate_cash_flows(cash_flows)?;

    let discount = 1.0 / (1.0 + rate);
    let value = discounted_sum(cash_flows.iter().rev(), discount);
    if value.is_finite() {
        return Ok(value);
    }

    // Scaling lets an intermediate sum exceed the f64 range when the final
    // result is still representable because a later addition cancels it.
    let scale = cash_flows
        .iter()
        .fold(0.0_f64, |max, flow| max.max(flow.abs()));
    let scaled = cash_flows
        .iter()
        .rev()
        .fold(0.0_f64, |sum, flow| sum.mul_add(discount, flow / scale));
    let value = scaled * scale;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CalcError::Numerical(
            "net present value exceeds the numeric range".into(),
        ))
    }
}

/// Internal rate of return per period. Zero flows are ignored when counting
/// sign changes. Exactly one sign change is required to ensure a unique rate.
pub fn irr(cash_flows: &[f64]) -> Result<f64, CalcError> {
    validate_cash_flows(cash_flows)?;
    let mut nonzero = cash_flows.iter().copied().filter(|flow| *flow != 0.0);
    let Some(first) = nonzero.next() else {
        return Err(CalcError::NoSolution(
            "IRR requires both inflows and outflows".into(),
        ));
    };
    let mut previous_sign = first.is_sign_positive();
    let mut sign_changes = 0;
    for flow in nonzero {
        let sign = flow.is_sign_positive();
        if sign != previous_sign {
            sign_changes += 1;
            previous_sign = sign;
        }
    }
    match sign_changes {
        0 => {
            return Err(CalcError::NoSolution(
                "IRR requires both inflows and outflows".into(),
            ))
        }
        1 => (),
        _ => {
            return Err(CalcError::NoSolution(
                "IRR requires a single cash-flow sign change to guarantee a unique result".into(),
            ))
        }
    }

    let start = cash_flows.iter().position(|flow| *flow != 0.0).unwrap_or(0);
    let end = cash_flows
        .iter()
        .rposition(|flow| *flow != 0.0)
        .unwrap_or(0);
    let cash_flows = &cash_flows[start..=end];

    // Search in log(1 + rate) to cover the representable rates near -1 as
    // well as large positive rates without overflowing the bracket.
    let lowest_rate = f64::from_bits((-1.0_f64).to_bits() - 1);
    let mut lower = lowest_rate.ln_1p();
    let mut upper = f64::MAX.ln();
    let mut lower_sign = irr_residual(cash_flows, lower);
    let upper_sign = irr_residual(cash_flows, upper);
    if lower_sign == 0.0 {
        return Ok(lowest_rate);
    }
    if upper_sign == 0.0 {
        return Ok(f64::MAX);
    }
    if lower_sign.is_sign_positive() == upper_sign.is_sign_positive() {
        return Err(CalcError::Numerical(
            "IRR lies outside the numeric range".into(),
        ));
    }

    let at_zero = irr_residual(cash_flows, 0.0);
    if at_zero == 0.0 {
        return Ok(0.0);
    }
    if at_zero.is_sign_positive() == lower_sign.is_sign_positive() {
        lower = 0.0;
        lower_sign = at_zero;
    } else {
        upper = 0.0;
    }

    for _ in 0..160 {
        let middle = lower + (upper - lower) / 2.0;
        if middle == lower || middle == upper {
            break;
        }
        let residual = irr_residual(cash_flows, middle);
        if residual == 0.0 {
            return Ok(middle.exp_m1());
        }
        if residual.is_sign_positive() == lower_sign.is_sign_positive() {
            lower = middle;
        } else {
            upper = middle;
        }
    }
    Ok((lower + (upper - lower) / 2.0).exp_m1())
}

fn validate_cash_flows(cash_flows: &[f64]) -> Result<(), CalcError> {
    if cash_flows.is_empty() || cash_flows.iter().any(|flow| !flow.is_finite()) {
        return Err(CalcError::InvalidInput(
            "provide at least one finite cash flow and no non-finite values".into(),
        ));
    }
    Ok(())
}

fn discounted_sum<'a>(flows: impl Iterator<Item = &'a f64>, factor: f64) -> f64 {
    flows.fold(0.0_f64, |sum, flow| sum.mul_add(factor, *flow))
}

fn irr_residual(cash_flows: &[f64], log_growth: f64) -> f64 {
    let factor = (-log_growth.abs()).exp();
    // Evaluate from the end with the greatest discount weight so that each
    // Horner step uses a multiplier in (0, 1]. The sign is what IRR needs.
    let value = if log_growth < 0.0 {
        discounted_sum(cash_flows.iter(), factor)
    } else {
        discounted_sum(cash_flows.iter().rev(), factor)
    };
    if value.is_finite() {
        return value;
    }

    let scale = cash_flows
        .iter()
        .fold(0.0_f64, |max, flow| max.max(flow.abs()));
    let step = |sum: f64, flow: &f64| sum.mul_add(factor, flow / scale);
    if log_growth < 0.0 {
        cash_flows.iter().fold(0.0_f64, step)
    } else {
        cash_flows.iter().rev().fold(0.0_f64, step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn amortizing_loan_tracks_payments_and_balance() {
        let terms = LoanTerms {
            principal: 1_000_000.0,
            annual_interest_rate: 0.06,
            amortization_months: 360,
            interest_only_months: 0,
        };
        let year_five = loan_metrics(&terms, 60).unwrap();
        close(year_five.monthly_payment, 5_995.505251527569, 1e-8);
        close(year_five.remaining_balance, 930_543.568228194, 1e-7);
        let final_month = loan_metrics(&terms, 359).unwrap();
        assert_eq!(final_month.annual_debt_service, final_month.monthly_payment);
        let paid_off = loan_metrics(&terms, 600).unwrap();
        assert_eq!(paid_off.remaining_balance, 0.0);
        assert_eq!(paid_off.annual_debt_service, 0.0);
        close(paid_off.interest_paid, 1_158_381.8905499252, 1e-6);
    }

    #[test]
    fn debt_service_accounts_for_interest_only_transition() {
        let terms = LoanTerms {
            principal: 300_000.0,
            annual_interest_rate: 0.06,
            amortization_months: 240,
            interest_only_months: 6,
        };
        let start = loan_metrics(&terms, 0).unwrap();
        let transition = loan_metrics(&terms, 6).unwrap();
        assert_eq!(start.monthly_payment, 1_500.0);
        assert_eq!(transition.interest_paid, 9_000.0);
        assert_eq!(transition.remaining_balance, terms.principal);
        close(
            start.annual_debt_service,
            9_000.0 + 6.0 * transition.monthly_payment,
            1e-9,
        );
    }

    #[test]
    fn fully_interest_only_and_zero_rate_loans_are_supported() {
        let terms = LoanTerms {
            principal: 12_000.0,
            annual_interest_rate: 0.12,
            amortization_months: 0,
            interest_only_months: 0,
        };
        let interest_only = loan_metrics(&terms, 60).unwrap();
        assert_eq!(interest_only.remaining_balance, 12_000.0);
        assert_eq!(interest_only.annual_debt_service, 1_440.0);
        let zero_rate = loan_metrics(
            &LoanTerms {
                annual_interest_rate: 0.0,
                amortization_months: 24,
                ..terms
            },
            18,
        )
        .unwrap();
        assert_eq!(zero_rate.remaining_balance, 3_000.0);
        assert_eq!(zero_rate.annual_debt_service, 3_000.0);
        assert_eq!(zero_rate.interest_paid, 0.0);
    }

    #[test]
    fn npv_counts_time_zero_and_handles_intermediate_overflow() {
        close(
            npv(0.1, &[-100.0, 60.0, 60.0]).unwrap(),
            4.13223140495867,
            1e-10,
        );
        assert_eq!(npv(0.0, &[1e308, -1e308, -1e308]).unwrap(), -1e308);
        assert!(matches!(npv(-1.0, &[1.0]), Err(CalcError::InvalidInput(_))));
    }

    #[test]
    fn irr_finds_positive_negative_and_zero_returns() {
        let flows = [-100.0, -30.0, 75.0, 100.0];
        let rate = irr(&flows).unwrap();
        close(npv(rate, &flows).unwrap(), 0.0, 1e-10);
        close(irr(&[0.0, -100.0, 80.0, 0.0]).unwrap(), -0.2, 1e-12);
        assert_eq!(irr(&[-100.0, 100.0]).unwrap(), 0.0);
    }

    #[test]
    fn irr_rejects_nonunique_and_undefined_cases() {
        assert!(matches!(
            irr(&[-100.0, 230.0, -132.0]),
            Err(CalcError::NoSolution(_))
        ));
        assert!(matches!(irr(&[0.0, 100.0]), Err(CalcError::NoSolution(_))));
        assert!(matches!(
            irr(&[f64::NAN, -100.0]),
            Err(CalcError::InvalidInput(_))
        ));
    }
}
