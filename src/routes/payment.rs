use crate::auth::nip98::Nip98Auth;
use crate::db::{Database, Payment};
use crate::payments::{Currency, PaymentAmount, PaymentInterval, PaymentUnit};
use crate::settings::Settings;
use chrono::{Months, Utc};
use fedimint_tonic_lnd::lnrpc::Invoice;
use fedimint_tonic_lnd::Client;
use log::{error, info};
use rocket::serde::json::Json;
use rocket::{routes, Route, State};
use serde::{Deserialize, Serialize};
use std::ops::{Add, Deref};

pub fn routes() -> Vec<Route> {
    routes![get_payment, req_payment]
}

#[derive(Deserialize, Serialize)]
struct PaymentInfo {
    /// Billing quota metric
    pub unit: PaymentUnit,

    /// Amount of time to bill units (GB/mo, Gb Egress/day etc.)
    pub interval: PaymentInterval,

    /// Value amount of payment
    pub cost: PaymentAmount,
}

#[derive(Deserialize, Serialize)]
struct PaymentRequest {
    /// Number of units requested to make payment
    pub units: f32,

    /// Quantity of orders to make
    pub quantity: u16,
}

#[derive(Deserialize, Serialize)]
struct PaymentResponse {
    pub pr: String,
}

#[rocket::get("/payment")]
async fn get_payment(settings: &State<Settings>) -> Option<Json<PaymentInfo>> {
    settings.payments.as_ref().map(|p| {
        Json::from(PaymentInfo {
            unit: p.unit.clone(),
            interval: p.interval.clone(),
            cost: p.cost.clone(),
        })
    })
}

#[rocket::post("/payment", data = "<req>", format = "json")]
async fn req_payment(
    auth: Nip98Auth,
    db: &State<Database>,
    settings: &State<Settings>,
    lnd: &State<Client>,
    req: Json<PaymentRequest>,
) -> Result<Json<PaymentResponse>, String> {
    let cfg = if let Some(p) = &settings.payments {
        p
    } else {
        return Err("Payment not enabled, missing configuration option(s)".to_string());
    };

    let btc_amount = match cfg.cost.currency {
        Currency::BTC => cfg.cost.amount,
        _ => return Err("Currency not supported".to_string()),
    };

    let amount = btc_amount * req.units * req.quantity as f32;

    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let uid = db
        .upsert_user(&pubkey_vec)
        .await
        .map_err(|_| "Failed to get user account".to_string())?;

    let mut lnd = lnd.deref().clone();
    let c = lnd.lightning();
    let msat = (amount * 1e11f32) as u64;
    let memo = format!(
        "{}x {} {} for {}",
        req.quantity, req.units, cfg.unit, auth.event.pubkey
    );
    info!("Requesting {} msats: {}", msat, memo);
    let invoice = c
        .add_invoice(Invoice {
            value_msat: msat as i64,
            memo,
            ..Default::default()
        })
        .await
        .map_err(|e| e.message().to_string())?;

    let days_value = match cfg.interval {
        PaymentInterval::Day(d) => d as u64,
        PaymentInterval::Month(m) => {
            let now = Utc::now();
            (now.add(Months::new(m as u32)) - now).num_days() as u64
        }
        PaymentInterval::Year(y) => {
            let now = Utc::now();
            (now.add(Months::new(12 * y as u32)) - now).num_days() as u64
        }
    };

    let record = Payment {
        payment_hash: invoice.get_ref().r_hash.clone(),
        user_id: uid,
        created: Default::default(),
        amount: msat,
        is_paid: false,
        days_value,
        size_value: cfg.unit.to_size(req.units),
        settle_index: None,
        rate: None,
    };

    if let Err(e) = db.insert_payment(&record).await {
        error!("Failed to insert payment: {}", e);
        return Err("Failed to insert payment".to_string());
    }

    Ok(Json(PaymentResponse {
        pr: invoice.get_ref().payment_request.clone(),
    }))
}
