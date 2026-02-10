use crate::db::Database;
use anyhow::Result;
use futures_util::StreamExt;
use log::{error, info};
use payments_rs::lightning::{InvoiceUpdate, LightningNode};
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct PaymentsHandler {
    node: Arc<dyn LightningNode>,
    database: Database,
}

impl PaymentsHandler {
    pub fn new(node: Arc<dyn LightningNode>, database: Database) -> Self {
        PaymentsHandler { node, database }
    }

    async fn process_payment(&self, update: &InvoiceUpdate) -> Result<()> {
        match update {
            InvoiceUpdate::Unknown { .. } => {}
            InvoiceUpdate::Error(_) => {}
            InvoiceUpdate::Created { .. } => {}
            InvoiceUpdate::Canceled { .. } => {}
            InvoiceUpdate::Settled { payment_hash, .. } => {
                let r_hash = hex::decode(payment_hash)?;
                if let Ok(Some(mut p)) = self.database.get_payment(&r_hash).await {
                    //p.settle_index = Some(msg.settle_index);
                    p.is_paid = true;
                    match self.database.complete_payment(&p).await {
                        Ok(()) => info!("Successfully completed payment: {}", payment_hash),
                        Err(e) => error!("Failed to complete payment: {}", e),
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn process(mut self, mut rx: broadcast::Receiver<()>) -> Result<()> {
        let last_invoice = self.database.get_last_settle_index().await?;
        let mut invoices = self
            .node
            .subscribe_invoices(last_invoice.as_ref().map(|i| i.1.clone()))
            .await?;
        info!(
            "Starting invoice subscription from {}",
            last_invoice.map(|i| i.0).unwrap_or_default()
        );

        loop {
            tokio::select! {
                Ok(_) = rx.recv() => {
                    break;
                }
                Some(msg) = invoices.next() => {
                    if let Err(e) = self.process_payment(&msg).await {
                        error!("Failed to process payment: {:?} {}", msg, e);
                    }
                }
            }
        }

        Ok(())
    }
}

impl Database {
    async fn get_last_settle_index(&self) -> Result<Option<(u64, Vec<u8>)>> {
        Ok(sqlx::query_as(
            "select max(settle_index),payment_hash from payments where is_paid = true",
        )
        .fetch_optional(&self.pool)
        .await?)
    }
}
