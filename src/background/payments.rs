use crate::db::Database;
use anyhow::Result;
use fedimint_tonic_lnd::Client;

pub struct PaymentsHandler {
    client: Client,
    database: Database,
}

impl PaymentsHandler {
    pub fn new(client: Client, database: Database) -> Self {
        PaymentsHandler { client, database }
    }

    pub async fn process(&mut self) -> Result<()> {
        Ok(())
    }
}
