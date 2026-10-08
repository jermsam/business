use anyhow::{ensure, Result};
use chrono::{DateTime, Duration, Months, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
pub fn text(value: &str, max: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control),
        "Invalid text"
    );
    Ok(())
}
pub fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|_| {
        dog_core::DogError::bad_request(
            "Invalid billing fields; card details are never accepted here",
        )
        .into_anyhow()
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomerInput {
    pub buyer_id: String,
    pub name: String,
    pub email: String,
    pub policy: Policy,
    pub manual_provider: Provider,
}
impl CustomerInput {
    pub fn validate(&self) -> Result<()> {
        id(&self.buyer_id)?;
        text(&self.name, 120)?;
        text(&self.email, 254)?;
        ensure!(
            self.email.contains('@') && !self.email.contains(['<', '>', ' ', '\"']),
            "Invalid email"
        );
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    pub request_id: String,
    pub customer_id: String,
    pub description: String,
    pub amount_cents: i64,
    pub start_at: String,
    pub interval: Interval,
    pub due_days: i64,
}
impl PlanInput {
    pub fn validate(&self, now: DateTime<Utc>) -> Result<i64> {
        id(&self.customer_id)?;
        text(&self.description, 200)?;
        ensure!(
            (50..=99_999_999).contains(&self.amount_cents),
            "USD amount must be 50..99999999 cents"
        );
        ensure!((0..=90).contains(&self.due_days), "Due days must be 0..90");
        let at = DateTime::parse_from_rfc3339(&self.start_at)?.with_timezone(&Utc);
        ensure!(
            at >= now - Duration::minutes(1) && at <= now + Duration::days(366 * 5),
            "Start must be now or within five years"
        );
        Ok(at.timestamp())
    }
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
    if requested == Mode::Automatic && consented && card_ready {
        Mode::Automatic
    } else {
        Mode::Manual
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn admin_cannot_invent_cardholder_consent() {
        for p in [Policy::Customer, Policy::Automatic] {
            assert_eq!(effective(p, Mode::Automatic, false, true), Mode::Manual);
            assert_eq!(effective(p, Mode::Automatic, true, false), Mode::Manual);
        }
        assert_eq!(
            effective(Policy::Manual, Mode::Automatic, true, true),
            Mode::Manual
        );
        assert_eq!(
            effective(Policy::Customer, Mode::Automatic, true, true),
            Mode::Automatic
        );
    }
    #[test]
    fn months_keep_anchor_and_once_ends() {
        let a = DateTime::parse_from_rfc3339("2027-01-31T15:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            DateTime::from_timestamp(occurrence(a, Interval::Month, 1).unwrap().unwrap(), 0)
                .unwrap()
                .to_rfc3339(),
            "2027-02-28T15:00:00+00:00"
        );
        assert_eq!(
            DateTime::from_timestamp(occurrence(a, Interval::Month, 2).unwrap().unwrap(), 0)
                .unwrap()
                .to_rfc3339(),
            "2027-03-31T15:00:00+00:00"
        );
        assert_eq!(occurrence(a, Interval::Once, 1).unwrap(), None);
    }
    #[test]
    fn reject_card_fields_and_fractional_cents() {
        assert!(parse::<CustomerInput>(json!({"buyer_id":uuid::Uuid::new_v4(),"name":"A","email":"a@b.com","policy":"customer","manual_provider":"mercury","card_number":"4242"})).is_err());
        assert!(parse::<PlanInput>(json!({"customer_id":uuid::Uuid::new_v4(),"description":"Work","amount_cents":1.2,"start_at":"2027-01-01T00:00:00Z","interval":"month","due_days":7})).is_err());
    }
}
