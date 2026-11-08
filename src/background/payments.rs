use crate::db::Database;
use anyhow::Result;
use futures_util::StreamExt;
use log::{error, info};
use payments_rs::lightning::{InvoiceUpdate, LightningNode};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

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

    pub async fn process(self, shutdown: CancellationToken) -> Result<()> {
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
                _ = shutdown.cancelled() => {
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
            "select settle_index,payment_hash from payments where is_paid = true and settle_index is not null order by settle_index desc limit 1",
        )
        .fetch_optional(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Payment;
    use async_trait::async_trait;
    use futures_util::stream;
    use payments_rs::lightning::{
        AddInvoiceRequest, AddInvoiceResponse, PayInvoiceRequest, PayInvoiceResponse,
    };
    use sqlx::Executor;
    use std::pin::Pin;

    struct FakeNode(Vec<InvoiceUpdate>);

    #[async_trait]
    impl LightningNode for FakeNode {
        async fn add_invoice(&self, _req: AddInvoiceRequest) -> Result<AddInvoiceResponse> {
            unimplemented!()
        }

        async fn cancel_invoice(&self, _id: &Vec<u8>) -> Result<()> {
            unimplemented!()
        }

        async fn pay_invoice(&self, _req: PayInvoiceRequest) -> Result<PayInvoiceResponse> {
            unimplemented!()
        }

        async fn subscribe_invoices(
            &self,
            _from_payment_hash: Option<Vec<u8>>,
        ) -> Result<Pin<Box<dyn futures_util::Stream<Item = InvoiceUpdate> + Send>>> {
            Ok(Box::pin(
                stream::iter(self.0.clone()).chain(stream::pending()),
            ))
        }
    }

    async fn test_db() -> Database {
        let url = std::env::var("ROUTE96_TEST_DATABASE")
            .expect("set ROUTE96_TEST_DATABASE=mysql://root:root@127.0.0.1:3399");
        let name = format!("r96_pay_{}", uuid::Uuid::new_v4().simple());
        let admin = Database::new(&url).await.unwrap();
        admin
            .pool
            .execute(format!("create database {}", name).as_str())
            .await
            .unwrap();
        let db = Database::new(&format!("{}/{}", url, name)).await.unwrap();
        db.migrate().await.unwrap();
        db
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn settled_invoice_marks_payment_paid() {
        let db = test_db().await;
        let uid = db.upsert_user(&vec![7u8; 32]).await.unwrap();
        let hash = vec![9u8; 32];
        db.insert_payment(&Payment {
            payment_hash: hash.clone(),
            user_id: uid,
            created: Default::default(),
            amount: 1000,
            is_paid: false,
            days_value: 30,
            size_value: 1024,
            settle_index: None,
            rate: None,
        })
        .await
        .unwrap();

        let node = FakeNode(vec![
            InvoiceUpdate::Canceled {
                payment_hash: hex::encode(vec![1u8; 32]),
            },
            InvoiceUpdate::Settled {
                payment_hash: hex::encode(&hash),
                preimage: None,
                external_id: None,
            },
        ]);
        let token = CancellationToken::new();
        let handler = PaymentsHandler::new(Arc::new(node), db.clone());
        let task = tokio::spawn(handler.process(token.clone()));

        let mut paid = false;
        for _ in 0..50 {
            paid = db.get_payment(&hash).await.unwrap().unwrap().is_paid;
            if paid {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        token.cancel();
        task.await.unwrap().unwrap();

        assert!(paid);
        let user = db.get_user_by_id(uid).await.unwrap();
        assert!(user.paid_until.is_some());
    }
}
