use super::{Candidate, CleanupHeuristic, RuleSetting, active_days, days_cutoff};
use crate::db::Database;
use crate::settings::Settings;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub struct Inactivity;

#[async_trait]
impl CleanupHeuristic for Inactivity {
    fn name(&self) -> &'static str {
        "inactive"
    }

    fn description(&self) -> &'static str {
        "Deletes files with no downloads within the configured number of days."
    }

    fn settings(&self, settings: &Settings) -> Vec<RuleSetting> {
        vec![RuleSetting::new(
            "delete_unaccessed_days",
            settings.delete_unaccessed_days,
        )]
    }

    fn enabled(&self, settings: &Settings) -> bool {
        active_days(settings.delete_unaccessed_days).is_some()
    }

    async fn candidates(
        &self,
        db: &Database,
        settings: &Settings,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<Candidate>> {
        let Some(days) = active_days(settings.delete_unaccessed_days) else {
            return Ok(vec![]);
        };
        let ids = db
            .get_unaccessed_files(days_cutoff(now, days), limit as u32)
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
        assert!(!Inactivity.enabled(&s));
        s.delete_unaccessed_days = Some(0);
        assert!(!Inactivity.enabled(&s));
        s.delete_unaccessed_days = Some(3);
        assert!(Inactivity.enabled(&s));
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn selects_old_unaccessed_files_only() {
        let db = test_db().await;
        let now = Utc::now();
        let old = add_file(
            &db,
            "old",
            "text/plain",
            now - chrono::Duration::days(10),
            None,
        )
        .await;
        add_file(
            &db,
            "new",
            "text/plain",
            now - chrono::Duration::days(1),
            None,
        )
        .await;
        let mut s = settings();
        s.delete_unaccessed_days = Some(5);
        let got = Inactivity.candidates(&db, &s, now, 100).await.unwrap();
        assert_eq!(
            got,
            vec![Candidate {
                id: old,
                reason: "inactive"
            }]
        );
    }
}
