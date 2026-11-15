use super::{Candidate, CleanupHeuristic, RuleSetting};
use crate::db::Database;
use crate::settings::Settings;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub struct FewUploadNonMedia;

fn max_files(settings: &Settings) -> Option<u64> {
    settings.delete_few_upload_max_files.filter(|&n| n > 0)
}

#[async_trait]
impl CleanupHeuristic for FewUploadNonMedia {
    fn name(&self) -> &'static str {
        "few_upload_non_media"
    }

    fn description(&self) -> &'static str {
        "Deletes non-media blobs (not image, video or audio) from pubkeys with few uploads. Admins and paid users are exempt."
    }

    fn settings(&self, settings: &Settings) -> Vec<RuleSetting> {
        vec![
            RuleSetting::new(
                "delete_few_upload_max_files",
                settings.delete_few_upload_max_files,
            ),
            RuleSetting::new(
                "delete_few_upload_min_age_hours",
                settings.delete_few_upload_min_age_hours(),
            ),
        ]
    }

    fn enabled(&self, settings: &Settings) -> bool {
        max_files(settings).is_some()
    }

    async fn candidates(
        &self,
        db: &Database,
        settings: &Settings,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<Candidate>> {
        let Some(max) = max_files(settings) else {
            return Ok(vec![]);
        };
        let hours = settings.delete_few_upload_min_age_hours();
        let cutoff = now - chrono::Duration::hours(hours as i64);
        let ids = db
            .get_few_upload_non_media_files(max, cutoff, now, limit as u32)
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
    fn disabled_when_none_or_zero() {
        let mut s = settings();
        assert!(!FewUploadNonMedia.enabled(&s));
        s.delete_few_upload_max_files = Some(0);
        assert!(!FewUploadNonMedia.enabled(&s));
        s.delete_few_upload_max_files = Some(3);
        assert!(FewUploadNonMedia.enabled(&s));
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn disabled_returns_nothing() {
        let db = test_db().await;
        let now = Utc::now();
        let user = add_user(&db, 1).await;
        add_file(
            &db,
            "spam",
            "text/plain",
            now - chrono::Duration::days(2),
            Some(user),
        )
        .await;
        let mut s = settings();
        assert!(
            FewUploadNonMedia
                .candidates(&db, &s, now, 100)
                .await
                .unwrap()
                .is_empty()
        );
        s.delete_few_upload_max_files = Some(0);
        assert!(
            FewUploadNonMedia
                .candidates(&db, &s, now, 100)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    #[ignore = "needs ROUTE96_TEST_DATABASE"]
    async fn selects_only_spam_like_blobs() {
        let db = test_db().await;
        let now = Utc::now();
        let old = now - chrono::Duration::days(2);

        let spammer = add_user(&db, 1).await;
        let spam = add_file(&db, "spam", "text/plain", old, Some(spammer)).await;

        let image_user = add_user(&db, 2).await;
        add_file(&db, "pic", "image/png", old, Some(image_user)).await;

        let heavy = add_user(&db, 3).await;
        for i in 0..3 {
            add_file(
                &db,
                &format!("heavy{i}"),
                "application/zip",
                old,
                Some(heavy),
            )
            .await;
        }

        let shared = add_file(&db, "shared", "text/plain", old, Some(spammer)).await;
        db.add_file(
            &crate::db::FileUpload {
                id: shared.clone(),
                mime_type: "text/plain".to_string(),
                created: old,
                ..Default::default()
            },
            Some(heavy),
        )
        .await
        .unwrap();

        let fresh_user = add_user(&db, 4).await;
        add_file(
            &db,
            "fresh",
            "text/plain",
            now - chrono::Duration::hours(1),
            Some(fresh_user),
        )
        .await;

        let admin = add_user(&db, 5).await;
        db.promote_to_admin(&vec![5u8; 32]).await.unwrap();
        add_file(&db, "admin", "text/plain", old, Some(admin)).await;

        let orphan = add_file(&db, "orphan", "text/plain", old, None).await;

        let mut s = settings();
        s.delete_few_upload_max_files = Some(3);
        let got = FewUploadNonMedia
            .candidates(&db, &s, now, 100)
            .await
            .unwrap();
        assert!(got.iter().all(|c| c.reason == "few_upload_non_media"));
        let ids = sorted(got.into_iter().map(|c| c.id).collect());
        assert_eq!(ids, sorted(vec![spam]));
        assert!(!ids.contains(&shared));
        assert!(!ids.contains(&orphan));
    }
}
