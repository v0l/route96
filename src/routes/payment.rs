use crate::auth::nip98::Nip98Auth;
use crate::db::Payment;
use crate::payments::{Currency, PaymentAmount, PaymentInterval, PaymentUnit};
use crate::routes::AppState;
use axum::{
    extract::State as AxumState,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{Months, Utc};
use fedimint_tonic_lnd::lnrpc::Invoice;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::ops::{Add, Deref};
use std::sync::Arc;

pub fn payment_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/payment", get(get_payment).post(req_payment))
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

async fn get_payment(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Result<Json<PaymentInfo>, StatusCode> {
    state
        .settings
        .payments
        .as_ref()
        .map(|p| {
            Json(PaymentInfo {
                unit: p.unit.clone(),
                interval: p.interval.clone(),
                cost: p.cost.clone(),
            })
        })
        .ok_or(StatusCode::NOT_FOUND)
}

async fn req_payment(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<PaymentRequest>,
) -> Result<Json<PaymentResponse>, (StatusCode, String)> {
    let cfg = if let Some(p) = &state.settings.payments {
        p
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Payment not enabled, missing configuration option(s)".to_string(),
        ));
    };

    let btc_amount = match cfg.cost.currency {
        Currency::BTC => cfg.cost.amount,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Currency not supported".to_string(),
            ))
        }
    };

    let amount = btc_amount * req.units * req.quantity as f32;

    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let uid = state
        .db
        .upsert_user(&pubkey_vec)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get user account".to_string(),
            )
        })?;

    let lnd_client = state
        .lnd
        .as_ref()
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "LND client not configured".to_string(),
        ))?;

    let mut lnd = lnd_client.deref().clone();
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
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()))?;

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

    if let Err(e) = state.db.insert_payment(&record).await {
        error!("Failed to insert payment: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to insert payment".to_string(),
        ));
    }

    Ok(Json(PaymentResponse {
        pr: invoice.get_ref().payment_request.clone(),
    }))
}
