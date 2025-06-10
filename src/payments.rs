use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[cfg(feature = "payments")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentAmount {
    pub currency: Currency,
    pub amount: f32,
}

#[cfg(feature = "payments")]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PaymentUnit {
    GBSpace,
    GBEgress,
}

impl PaymentUnit {
    /// Get the total size from a number of units
    pub fn to_size(&self, units: f32) -> u64 {
        (1000f32 * 1000f32 * 1000f32 * units) as u64
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaymentInterval {
    Day(u16),
    Month(u16),
    Year(u16),
}
