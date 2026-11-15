use super::{Candidate, CleanupHeuristic, active_days, days_cutoff};
use crate::db::Database;
use crate::settings::Settings;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub struct ZeroEgress;

#[async_trait]
impl CleanupHeuristic for ZeroEgress {
    fn name(&self) -> &'static str {
        "zero_egress"
    }

    fn enabled(&self, settings: &Settings) -> bool {
        active_days(settings.delete_zero_egress_days).is_some()
    }

    async fn candidates(
        &self,
        db: &Database,
        settings: &Settings,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<Candidate>> {
        let Some(days) = active_days(settings.delete_zero_egress_days) else {
            return Ok(vec![]);
        };
        let ids = db
            .get_files_with_zero_egress(days_cutoff(now, days), limit as u32)
            .await?;
        Ok(ids
            .into_iter()
            .map(|id| Candidate {
                id,
                reason: self.name(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::cleanup::test_support::*;

    #[test]
    fn enabled_only_for_positive_days() {
        let mut s = settings();
        assert!(!ZeroEgress.enabled(&s));
        s.delete_zero_egress_days = Some(0);
        assert!(!ZeroEgress.enabled(&s));
        s.delete_zero_egress_days = Some(3);
        assert!(ZeroEgress.enabled(&s));
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn selects_old_never_downloaded_files() {
        let db = test_db().await;
        let now = Utc::now();
        let old = add_file(
            &db,
            "old",
            "image/png",
            now - chrono::Duration::days(10),
            None,
        )
        .await;
        add_file(
            &db,
            "new",
            "image/png",
            now - chrono::Duration::days(1),
            None,
        )
        .await;
        let mut s = settings();
        s.delete_zero_egress_days = Some(5);
        let got = ZeroEgress.candidates(&db, &s, now, 100).await.unwrap();
        assert_eq!(
            got,
            vec![Candidate {
                id: old,
                reason: "zero_egress"
            }]
        );
    }
}
