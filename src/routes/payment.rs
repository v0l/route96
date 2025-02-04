use crate::auth::nip98::Nip98Auth;
use crate::db::Database;
use crate::settings::{PaymentAmount, PaymentInterval, PaymentUnit, Settings};
use rocket::serde::json::Json;
use rocket::{routes, Route, State};
use serde::{Deserialize, Serialize};

pub fn routes() -> Vec<Route> {
    routes![get_payment]
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
struct PaymentResponse {}

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
    req: Json<PaymentRequest>,
) -> Json<PaymentResponse> {
    Json::from(PaymentResponse {})
}
