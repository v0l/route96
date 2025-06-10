use crate::db::Database;
use anyhow::Result;
use fedimint_tonic_lnd::lnrpc::invoice::InvoiceState;
use fedimint_tonic_lnd::lnrpc::InvoiceSubscription;
use fedimint_tonic_lnd::Client;
use log::{error, info};
use rocket::futures::StreamExt;
use sqlx::Row;
use tokio::sync::broadcast;

pub struct PaymentsHandler {
    client: Client,
    database: Database,
}

impl PaymentsHandler {
    pub fn new(client: Client, database: Database) -> Self {
        PaymentsHandler { client, database }
    }

    pub async fn process(&mut self, mut rx: broadcast::Receiver<()>) -> Result<()> {
        let start_idx = self.database.get_last_settle_index().await?;
        let mut invoices = self
            .client
            .lightning()
            .subscribe_invoices(InvoiceSubscription {
                add_index: 0,
                settle_index: start_idx,
            })
            .await?;
        info!("Starting invoice subscription from {}", start_idx);

        let invoices = invoices.get_mut();
        loop {
            tokio::select! {
                Ok(_) = rx.recv() => {
                    break;
                }
                Some(Ok(msg)) = invoices.next() => {
                    if msg.state == InvoiceState::Settled as i32 {
                        if let Ok(Some(mut p)) = self.database.get_payment(&msg.r_hash).await {
                            p.settle_index = Some(msg.settle_index);
                            p.is_paid = true;
                            match self.database.complete_payment(&p).await {
                                Ok(()) => info!(
                                    "Successfully completed payment: {}",
                                    hex::encode(&msg.r_hash)
                                ),
                                Err(e) => error!("Failed to complete payment: {}", e),
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Database {
    async fn get_last_settle_index(&self) -> Result<u64> {
        Ok(
            sqlx::query("select max(settle_index) from payments where is_paid = true")
                .fetch_one(&self.pool)
                .await?
                .try_get(0)
                .unwrap_or(0),
        )
    }
}
