use crate::db::{Database, Payment};
use crate::settings::Settings;
use anyhow::Result;
use futures_util::StreamExt;
use log::{error, info, warn};
use payments_rs::lightning::{InvoiceUpdate, LightningNode};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

const DEFAULT_REPAIR_DAYS: u64 = 7;
const MIN_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

pub struct PaymentsHandler {
    node: Arc<dyn LightningNode>,
    database: Database,
    settings: Option<Arc<RwLock<Settings>>>,
}

impl PaymentsHandler {
    pub fn new(node: Arc<dyn LightningNode>, database: Database) -> Self {
        PaymentsHandler {
            node,
            database,
            settings: None,
        }
    }

    pub fn with_settings(mut self, settings: Arc<RwLock<Settings>>) -> Self {
        self.settings = Some(settings);
        self
    }

    async fn repair_days(&self) -> u64 {
        match &self.settings {
            Some(s) => s
                .read()
                .await
                .payment_repair_days
                .unwrap_or(DEFAULT_REPAIR_DAYS),
            None => DEFAULT_REPAIR_DAYS,
        }
    }

    async fn settle(&self, mut payment: Payment, settle_index: Option<u64>) -> Result<bool> {
        payment.settle_index = settle_index;
        payment.is_paid = true;
        let credited = self.database.complete_payment(&payment).await?;
        if credited {
            info!(
                "Completed payment {} at settle index {:?}",
                hex::encode(&payment.payment_hash),
                settle_index
            );
        }
        Ok(credited)
    }

    async fn process_payment(&self, update: &InvoiceUpdate) -> Result<()> {
        if let InvoiceUpdate::Settled {
            payment_hash,
            settle_index,
            ..
        } = update
        {
            let r_hash = hex::decode(payment_hash)?;
            if let Some(p) = self.database.get_payment(&r_hash).await? {
                self.settle(p, *settle_index).await?;
            }
        }
        Ok(())
    }

    async fn repair_missed(&self, shutdown: &CancellationToken) -> Result<usize> {
        let days = self.repair_days().await;
        if days == 0 {
            return Ok(0);
        }
        let unpaid = self.database.get_unpaid_payments_since(days).await?;
        let mut repaired = 0;
        for p in unpaid {
            if shutdown.is_cancelled() {
                break;
            }
            let hash = hex::encode(&p.payment_hash);
            match self.node.lookup_invoice(&p.payment_hash).await {
                Ok(InvoiceUpdate::Settled { settle_index, .. }) => {
                    match self.settle(p, settle_index).await {
                        Ok(true) => repaired += 1,
                        Ok(false) => {}
                        Err(e) => error!("Failed to complete repaired payment {}: {}", hash, e),
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("Failed to look up invoice {}: {}", hash, e),
            }
        }
        Ok(repaired)
    }

    async fn run_subscription(&self, shutdown: &CancellationToken) -> Result<bool> {
        let last_invoice = self.database.get_last_settle_index().await?;
        let mut invoices = self
            .node
            .subscribe_invoices(last_invoice.as_ref().map(|i| i.1.clone()))
            .await?;
        info!(
            "Starting invoice subscription from {}",
            last_invoice.map(|i| i.0).unwrap_or_default()
        );

        match self.repair_missed(shutdown).await {
            Ok(0) => {}
            Ok(n) => info!("Credited {} payments missed by the invoice stream", n),
            Err(e) => error!("Failed to repair missed payments: {}", e),
        }

        let mut received = false;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(received),
                msg = invoices.next() => match msg {
                    Some(InvoiceUpdate::Error(e)) => {
                        warn!("Invoice stream error: {}", e);
                        return Ok(received);
                    }
                    Some(msg) => {
                        received = true;
                        if let Err(e) = self.process_payment(&msg).await {
                            error!("Failed to process payment: {:?} {}", msg, e);
                        }
                    }
                    None => {
                        warn!("Invoice stream ended");
                        return Ok(received);
                    }
                }
            }
        }
    }

    pub async fn process(self, shutdown: CancellationToken) -> Result<()> {
        let mut backoff = MIN_BACKOFF;
        while !shutdown.is_cancelled() {
            match self.run_subscription(&shutdown).await {
                Ok(true) => backoff = MIN_BACKOFF,
                Ok(false) => {}
                Err(e) => error!("Invoice subscription failed: {}", e),
            }
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
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
    use std::collections::{HashMap, VecDeque};
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::time::Duration;

    enum Subscription {
        Fail,
        Ends(Vec<InvoiceUpdate>),
        Stays(Vec<InvoiceUpdate>),
    }

    #[derive(Default)]
    struct FakeNode {
        subscriptions: Mutex<VecDeque<Subscription>>,
        subscribed_from: Mutex<Vec<Option<Vec<u8>>>>,
        invoices: HashMap<Vec<u8>, Result<InvoiceUpdate, String>>,
        looked_up: Mutex<Vec<Vec<u8>>>,
    }

    impl FakeNode {
        fn with_subscriptions(subs: Vec<Subscription>) -> Self {
            FakeNode {
                subscriptions: Mutex::new(subs.into()),
                ..Default::default()
            }
        }

        fn invoice(mut self, hash: &[u8], update: Result<InvoiceUpdate, String>) -> Self {
            self.invoices.insert(hash.to_vec(), update);
            self
        }

        fn subscribed_from(&self) -> Vec<Option<Vec<u8>>> {
            self.subscribed_from.lock().unwrap().clone()
        }

        fn looked_up(&self) -> Vec<Vec<u8>> {
            self.looked_up.lock().unwrap().clone()
        }
    }

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
            from_payment_hash: Option<Vec<u8>>,
        ) -> Result<Pin<Box<dyn futures_util::Stream<Item = InvoiceUpdate> + Send>>> {
            self.subscribed_from.lock().unwrap().push(from_payment_hash);
            let next = self.subscriptions.lock().unwrap().pop_front();
            match next {
                Some(Subscription::Fail) => Err(anyhow::anyhow!("connection refused")),
                Some(Subscription::Ends(updates)) => Ok(Box::pin(stream::iter(updates))),
                Some(Subscription::Stays(updates)) => {
                    Ok(Box::pin(stream::iter(updates).chain(stream::pending())))
                }
                None => Ok(Box::pin(stream::pending())),
            }
        }

        async fn lookup_invoice(&self, payment_hash: &[u8]) -> Result<InvoiceUpdate> {
            self.looked_up.lock().unwrap().push(payment_hash.to_vec());
            match self.invoices.get(payment_hash) {
                Some(Ok(u)) => Ok(u.clone()),
                Some(Err(e)) => Err(anyhow::anyhow!(e.clone())),
                None => Err(anyhow::anyhow!("unable to locate invoice")),
            }
        }
    }

    fn settled(hash: &[u8], settle_index: Option<u64>) -> InvoiceUpdate {
        InvoiceUpdate::Settled {
            payment_hash: hex::encode(hash),
            preimage: None,
            external_id: None,
            settle_index,
        }
    }

    async fn test_db() -> Database {
        let db = crate::db::strict_mode_tests::strict_db().await;
        db.migrate().await.unwrap();
        db
    }

    async fn add_payment(db: &Database, user: u8, hash: &[u8]) -> u64 {
        let uid = db.upsert_user(&vec![user; 32]).await.unwrap();
        db.insert_payment(&Payment {
            payment_hash: hash.to_vec(),
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
        uid
    }

    async fn payment(db: &Database, hash: &[u8]) -> Payment {
        db.get_payment(&hash.to_vec()).await.unwrap().unwrap()
    }

    async fn wait_for<F, Fut>(mut check: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..80 {
            if check().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    fn start(
        node: Arc<FakeNode>,
        db: &Database,
    ) -> (CancellationToken, tokio::task::JoinHandle<Result<()>>) {
        let token = CancellationToken::new();
        let handler = PaymentsHandler::new(node, db.clone());
        (token.clone(), tokio::spawn(handler.process(token)))
    }

    async fn stop(token: CancellationToken, task: tokio::task::JoinHandle<Result<()>>) {
        token.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("handler did not stop on shutdown")
            .unwrap()
            .unwrap();
    }

    fn days_paid(paid_until: Option<chrono::DateTime<chrono::Utc>>) -> i64 {
        (paid_until.expect("paid_until set") - chrono::Utc::now()).num_hours() / 24
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn settled_invoice_marks_payment_paid() {
        let db = test_db().await;
        let hash = vec![9u8; 32];
        let uid = add_payment(&db, 7, &hash).await;

        let node = Arc::new(FakeNode::with_subscriptions(vec![Subscription::Stays(
            vec![
                InvoiceUpdate::Canceled {
                    payment_hash: hex::encode(vec![1u8; 32]),
                },
                settled(&hash, Some(1)),
            ],
        )]));
        let (token, task) = start(node, &db);
        let paid = wait_for(|| async { payment(&db, &hash).await.is_paid }).await;
        stop(token, task).await;

        assert!(paid);
        let user = db.get_user_by_id(uid).await.unwrap();
        assert!(user.paid_until.is_some());
        assert_eq!(user.paid_size, 1024);
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn stream_settle_stores_settle_index_and_next_subscribe_resumes_from_it() {
        let db = test_db().await;
        let hash = vec![0xa1u8; 32];
        add_payment(&db, 1, &hash).await;

        let node = Arc::new(FakeNode::with_subscriptions(vec![Subscription::Ends(
            vec![settled(&hash, Some(5))],
        )]));
        let (token, task) = start(node.clone(), &db);
        let resubscribed = wait_for(|| async { node.subscribed_from().len() >= 2 }).await;
        stop(token, task).await;

        let p = payment(&db, &hash).await;
        assert!(p.is_paid);
        assert_eq!(p.settle_index, Some(5));
        assert!(resubscribed, "subscriptions: {:?}", node.subscribed_from());
        assert_eq!(node.subscribed_from()[0], None);
        assert_eq!(node.subscribed_from()[1], Some(hash));
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn settle_missed_while_down_is_credited_once_on_start() {
        let db = test_db().await;
        let missed = vec![0xb1u8; 32];
        let broken = vec![0xb2u8; 32];
        let uid = add_payment(&db, 2, &missed).await;
        add_payment(&db, 3, &broken).await;

        let node = Arc::new(
            FakeNode::with_subscriptions(vec![])
                .invoice(&broken, Err("rpc timeout".to_string()))
                .invoice(&missed, Ok(settled(&missed, Some(11)))),
        );
        let (token, task) = start(node.clone(), &db);
        let paid = wait_for(|| async { payment(&db, &missed).await.is_paid }).await;
        stop(token, task).await;

        assert!(paid, "lookups: {:?}", node.looked_up());
        assert_eq!(payment(&db, &missed).await.settle_index, Some(11));
        assert!(!payment(&db, &broken).await.is_paid);
        let user = db.get_user_by_id(uid).await.unwrap();
        assert_eq!(user.paid_size, 1024);
        assert!((29..=30).contains(&days_paid(user.paid_until)));

        let node = Arc::new(
            FakeNode::with_subscriptions(vec![]).invoice(&missed, Ok(settled(&missed, Some(11)))),
        );
        let (token, task) = start(node.clone(), &db);
        tokio::time::sleep(Duration::from_millis(500)).await;
        stop(token, task).await;
        assert!(!node.looked_up().contains(&missed));
        let user = db.get_user_by_id(uid).await.unwrap();
        assert!((29..=30).contains(&days_paid(user.paid_until)));
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn stream_end_resubscribes_from_stored_index() {
        let db = test_db().await;
        let old = vec![0xc1u8; 32];
        let newer = vec![0xc2u8; 32];
        add_payment(&db, 4, &old).await;
        add_payment(&db, 4, &newer).await;
        sqlx::query("update payments set is_paid = true, settle_index = ? where payment_hash = ?")
            .bind(3u64)
            .bind(&old)
            .execute(&db.pool)
            .await
            .unwrap();

        let node = Arc::new(FakeNode::with_subscriptions(vec![
            Subscription::Ends(vec![]),
            Subscription::Fail,
            Subscription::Ends(vec![settled(&newer, Some(8))]),
        ]));
        let (token, task) = start(node.clone(), &db);
        let resubscribed = wait_for(|| async { node.subscribed_from().len() >= 4 }).await;
        stop(token, task).await;

        assert!(resubscribed, "subscriptions: {:?}", node.subscribed_from());
        let from = node.subscribed_from();
        assert_eq!(from[0], Some(old.clone()));
        assert_eq!(from[1], Some(old.clone()));
        assert_eq!(from[2], Some(old));
        assert_eq!(from[3], Some(newer.clone()));
        assert_eq!(payment(&db, &newer).await.settle_index, Some(8));
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn replayed_settle_does_not_credit_twice() {
        let db = test_db().await;
        let hash = vec![0xd1u8; 32];
        let uid = add_payment(&db, 5, &hash).await;

        let node = Arc::new(
            FakeNode::with_subscriptions(vec![
                Subscription::Ends(vec![settled(&hash, Some(4)), settled(&hash, Some(4))]),
                Subscription::Stays(vec![settled(&hash, Some(4))]),
            ])
            .invoice(&hash, Ok(settled(&hash, Some(4)))),
        );
        let (token, task) = start(node.clone(), &db);
        wait_for(|| async { node.subscribed_from().len() >= 2 }).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        stop(token, task).await;

        assert!(payment(&db, &hash).await.is_paid);
        let user = db.get_user_by_id(uid).await.unwrap();
        assert_eq!(user.paid_size, 1024);
        assert!((29..=30).contains(&days_paid(user.paid_until)));
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn expired_unpaid_payment_is_left_alone() {
        let db = test_db().await;
        let stale = vec![0xe1u8; 32];
        let expired = vec![0xe2u8; 32];
        let fresh = vec![0xe3u8; 32];
        let stale_uid = add_payment(&db, 6, &stale).await;
        let expired_uid = add_payment(&db, 8, &expired).await;
        add_payment(&db, 9, &fresh).await;
        sqlx::query("update payments set created = TIMESTAMPADD(DAY, -30, current_timestamp) where payment_hash = ?")
            .bind(&stale)
            .execute(&db.pool)
            .await
            .unwrap();

        let node = Arc::new(
            FakeNode::with_subscriptions(vec![])
                .invoice(&stale, Ok(settled(&stale, Some(2))))
                .invoice(
                    &expired,
                    Ok(InvoiceUpdate::Canceled {
                        payment_hash: hex::encode(&expired),
                    }),
                )
                .invoice(&fresh, Ok(settled(&fresh, Some(3)))),
        );
        let (token, task) = start(node.clone(), &db);
        let repaired = wait_for(|| async { payment(&db, &fresh).await.is_paid }).await;
        stop(token, task).await;

        assert!(repaired, "lookups: {:?}", node.looked_up());
        assert!(node.looked_up().contains(&expired));
        assert!(!node.looked_up().contains(&stale));
        assert!(!payment(&db, &stale).await.is_paid);
        assert!(!payment(&db, &expired).await.is_paid);
        assert!(
            db.get_user_by_id(stale_uid)
                .await
                .unwrap()
                .paid_until
                .is_none()
        );
        assert!(
            db.get_user_by_id(expired_uid)
                .await
                .unwrap()
                .paid_until
                .is_none()
        );
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn repair_window_follows_live_setting() {
        let db = test_db().await;
        let old = vec![0xf1u8; 32];
        add_payment(&db, 10, &old).await;
        sqlx::query("update payments set created = TIMESTAMPADD(DAY, -30, current_timestamp) where payment_hash = ?")
            .bind(&old)
            .execute(&db.pool)
            .await
            .unwrap();
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "storage_dir": "/tmp",
            "database": "mysql://localhost/test",
            "max_upload_bytes": 1,
            "public_url": "https://example.com",
            "payment_repair_days": 40,
        }))
        .unwrap();

        let node = Arc::new(
            FakeNode::with_subscriptions(vec![]).invoice(&old, Ok(settled(&old, Some(6)))),
        );
        let token = CancellationToken::new();
        let handler = PaymentsHandler::new(node.clone(), db.clone())
            .with_settings(Arc::new(RwLock::new(settings)));
        let task = tokio::spawn(handler.process(token.clone()));
        let paid = wait_for(|| async { payment(&db, &old).await.is_paid }).await;
        stop(token, task).await;

        assert!(paid, "lookups: {:?}", node.looked_up());
    }
}
