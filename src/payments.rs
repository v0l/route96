use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[cfg(feature = "payments")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaymentAmount {
    pub currency: Currency,
    pub amount: f32,
}

#[cfg(feature = "payments")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Currency {
    BTC,
    USD,
    EUR,
    GBP,
    JPY,
    CAD,
    AUD,
}

#[cfg(feature = "payments")]
impl Display for Currency {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[cfg(feature = "payments")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PaymentUnit {
    GBSpace,
    GBEgress,
}

impl PaymentUnit {
    /// Get the total size from a number of units
    pub fn to_size(&self, units: f32) -> u64 {
        (1024f32 * 1024f32 * 1024f32 * units) as u64
    }
}

impl Display for PaymentUnit {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentUnit::GBSpace => write!(f, "GB Space"),
            PaymentUnit::GBEgress => write!(f, "GB Egress"),
        }
    }
}

#[cfg(feature = "payments")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaymentInterval {
    Day(u16),
    Month(u16),
    Year(u16),
}

#[cfg(feature = "payments")]
impl PaymentInterval {
    pub fn label(&self) -> String {
        match self {
            PaymentInterval::Day(1) => "daily".to_string(),
            PaymentInterval::Day(n) => format!("every {n} days"),
            PaymentInterval::Month(1) => "monthly".to_string(),
            PaymentInterval::Month(n) => format!("every {n} months"),
            PaymentInterval::Year(1) => "yearly".to_string(),
            PaymentInterval::Year(n) => format!("every {n} years"),
        }
    }
}

#[cfg(all(test, feature = "payments"))]
mod tests {
    use super::*;

    #[test]
    fn interval_labels_keep_legacy_wire_values() {
        assert_eq!(PaymentInterval::Month(1).label(), "monthly");
        assert_eq!(PaymentInterval::Year(1).label(), "yearly");
        assert_eq!(PaymentInterval::Day(1).label(), "daily");
        assert_eq!(PaymentInterval::Month(3).label(), "every 3 months");
        assert_eq!(PaymentInterval::Year(2).label(), "every 2 years");
        assert_eq!(PaymentInterval::Day(7).label(), "every 7 days");
    }

    #[test]
    fn currency_displays_code() {
        assert_eq!(Currency::BTC.to_string(), "BTC");
        assert_eq!(Currency::USD.to_string(), "USD");
    }
}
