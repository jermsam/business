use anyhow::Result;
use chrono::{DateTime, Duration, Months};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Manual,
    Automatic,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    Customer,
    Manual,
    Automatic,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Mercury,
    Stripe,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Interval {
    Once,
    Week,
    Month,
    Year,
}
pub fn word<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
pub fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}
pub fn id(value: &str) -> Result<String> {
    Ok(uuid::Uuid::parse_str(value)?.to_string())
}
/// Recompute from the original anchor, so Jan 31 -> Feb 28 -> Mar 31.
/// Schedules use UTC, explicitly; no hidden DST or server-local timezone conversion.
pub fn occurrence(anchor: i64, interval: Interval, sequence: u32) -> Result<Option<i64>> {
    let at =
        DateTime::from_timestamp(anchor, 0).ok_or_else(|| anyhow::anyhow!("Invalid anchor"))?;
    let next = match interval {
        Interval::Once => {
            if sequence == 0 {
                Some(at)
            } else {
                None
            }
        }
        Interval::Week => Some(
            at.checked_add_signed(Duration::days(i64::from(sequence) * 7))
                .ok_or_else(|| anyhow::anyhow!("Schedule overflow"))?,
        ),
        Interval::Month | Interval::Year => Some(
            at.checked_add_months(Months::new(
                sequence
                    .checked_mul(if interval == Interval::Year { 12 } else { 1 })
                    .ok_or_else(|| anyhow::anyhow!("Schedule overflow"))?,
            ))
            .ok_or_else(|| anyhow::anyhow!("Schedule overflow"))?,
        ),
    };
    Ok(next.map(|t| t.timestamp()))
}
/// An admin preference is not cardholder consent. Missing consent always falls
/// back to a manual invoice; choosing automatic never fabricates a saved card.
pub fn effective(policy: Policy, choice: Mode, consented: bool, card_ready: bool) -> Mode {
    let requested = match policy {
        Policy::Manual => Mode::Manual,
        Policy::Automatic => Mode::Automatic,
        Policy::Customer => choice,
    };
    if requested == Mode::Automatic && choice == Mode::Automatic && consented && card_ready {
        Mode::Automatic
    } else {
        Mode::Manual
    }
}
#[cfg(test)]
#[path = "../../tests/billing/billing_schema.rs"]
mod tests;
