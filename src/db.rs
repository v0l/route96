use crate::comma_separated::CommaSeparated;
use crate::filesystem::NewFileResult;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::migrate::MigrateError;
use sqlx::{Error, Executor, FromRow, Row};

/// Review/moderation state for an uploaded file.
///
/// Stored as a `TINYINT UNSIGNED` in MySQL (0–3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[repr(u16)]
pub enum ReviewState {
    /// No review needed (default). Value = 0.
    #[default]
    None = 0,
    /// Auto-flagged because AI labels matched a configured term. Value = 1.
    LabelFlagged = 1,
    /// Flagged because a user submitted a report. Value = 2.
    Reported = 2,
    /// An admin has reviewed the file and cleared it. Value = 3.
    Reviewed = 3,
}

#[derive(Clone, FromRow, Default, Serialize)]
pub struct FileUpload {
    /// SHA-256 hash of the file
    #[serde(with = "hex")]
    pub id: Vec<u8>,
    /// Filename
    pub name: Option<String>,
    /// Size in bytes
    pub size: u64,
    /// MIME type
    pub mime_type: String,
    /// When the upload was created
    pub created: DateTime<Utc>,
    /// Width of the media in pixels
    pub width: Option<u32>,
    /// Height of the media in pixels
    pub height: Option<u32>,
    /// Blurhash of the media
    pub blur_hash: Option<String>,
    /// Alt text of the media
    pub alt: Option<String>,
    /// Duration of media in seconds
    pub duration: Option<f32>,
    /// Average bitrate in bits/s
    pub bitrate: Option<u32>,
    /// Moderation review state
    pub review_state: ReviewState,
    /// When true the file has been admin-deleted and re-uploads must be rejected.
    pub banned: bool,
    /// Comma-separated list of model names that have already labeled this file.
    #[cfg(feature = "labels")]
    pub labeled_by: CommaSeparated<String>,

    #[sqlx(skip)]
    #[cfg(feature = "labels")]
    pub labels: Vec<FileLabel>,
}

impl From<&NewFileResult> for FileUpload {
    fn from(value: &NewFileResult) -> Self {
        Self {
            id: value.id.clone(),
            name: None,
            size: value.size,
            mime_type: value.mime_type.clone(),
            created: Utc::now(),
            width: value.width,
            height: value.height,
            blur_hash: value.blur_hash.clone(),
            alt: None,
            duration: value.duration,
            bitrate: value.bitrate,
            review_state: ReviewState::None,
            banned: false,
            #[cfg(feature = "labels")]
            labeled_by: CommaSeparated::default(),
            #[cfg(feature = "labels")]
            labels: value.labels.clone(),
        }
    }
}
#[derive(Clone, FromRow, Serialize)]
pub struct User {
    pub id: u64,
    #[serde(with = "hex")]
    pub pubkey: Vec<u8>,
    pub created: DateTime<Utc>,
    pub is_admin: bool,
    #[cfg(feature = "payments")]
    pub paid_until: Option<DateTime<Utc>>,
    #[cfg(feature = "payments")]
    pub paid_size: u64,
}

#[cfg(feature = "labels")]
#[derive(Clone, FromRow, Serialize)]
pub struct FileLabel {
    pub file: Vec<u8>,
    pub label: String,
    pub created: DateTime<Utc>,
    pub model: String,
}

#[cfg(feature = "labels")]
impl FileLabel {
    pub fn new(label: String, model: String) -> Self {
        Self {
            file: vec![],
            label,
            created: Utc::now(),
            model,
        }
    }
}

#[derive(Clone, FromRow, Serialize)]
pub struct UserStats {
    pub file_count: u64,
    pub total_size: u64,
}

#[cfg(feature = "payments")]
#[derive(Clone, FromRow, Serialize)]
pub struct Payment {
    pub payment_hash: Vec<u8>,
    pub user_id: u64,
    pub created: DateTime<Utc>,
    pub amount: u64,
    pub is_paid: bool,
    pub days_value: u64,
    pub size_value: u64,
    pub settle_index: Option<u64>,
    pub rate: Option<f32>,
}

#[derive(Clone, FromRow, Serialize)]
pub struct Report {
    pub id: u64,
    #[serde(with = "hex")]
    pub file_id: Vec<u8>,
    pub reporter_id: u64,
    pub event_json: String,
    pub created: DateTime<Utc>,
    pub reviewed: bool,
}

#[derive(Clone)]
pub struct Database {
    pub(crate) pool: sqlx::pool::Pool<sqlx::mysql::MySql>,
}

impl Database {
    pub async fn new(conn: &str) -> Result<Self, Error> {
        let db = sqlx::mysql::MySqlPool::connect(conn).await?;
        Ok(Self { pool: db })
    }

    pub async fn migrate(&self) -> Result<(), MigrateError> {
        sqlx::migrate!("./migrations/").run(&self.pool).await
    }

    pub async fn upsert_user(&self, pubkey: &Vec<u8>) -> Result<u64, Error> {
        let res = sqlx::query("insert ignore into users(pubkey) values(?) returning id")
            .bind(pubkey)
            .fetch_optional(&self.pool)
            .await?;
        let user_id = match res {
            None => sqlx::query("select id from users where pubkey = ?")
                .bind(pubkey)
                .fetch_one(&self.pool)
                .await?
                .try_get(0)?,
            Some(res) => res.try_get(0)?,
        };

        // Make the first user (ID 1) an admin
        if user_id == 1 {
            sqlx::query("update users set is_admin = 1 where id = 1")
                .execute(&self.pool)
                .await?;
        }

        Ok(user_id)
    }

    pub async fn get_user(&self, pubkey: &Vec<u8>) -> Result<User, Error> {
        sqlx::query_as("select * from users where pubkey = ?")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn get_user_by_id(&self, user_id: u64) -> Result<User, Error> {
        sqlx::query_as("select * from users where id = ?")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn get_user_stats(&self, id: u64) -> Result<UserStats, Error> {
        sqlx::query_as(
            "select cast(count(user_uploads.file) as unsigned integer) as file_count, \
        cast(coalesce(sum(uploads.size), 0) as unsigned integer) as total_size \
        from user_uploads,uploads \
        where user_uploads.user_id = ? \
        and user_uploads.file = uploads.id",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_user_id(&self, pubkey: &Vec<u8>) -> Result<u64, Error> {
        sqlx::query("select id from users where pubkey = ?")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await?
            .try_get(0)
    }

    pub async fn add_file(&self, file: &FileUpload, user_id: Option<u64>) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        let q = sqlx::query("insert ignore into \
        uploads(id,name,size,mime_type,blur_hash,width,height,alt,created,duration,bitrate,review_state,banned,labeled_by) values(?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(&file.id)
            .bind(&file.name)
            .bind(file.size)
            .bind(&file.mime_type)
            .bind(&file.blur_hash)
            .bind(file.width)
            .bind(file.height)
            .bind(&file.alt)
            .bind(file.created)
            .bind(file.duration)
            .bind(file.bitrate)
            .bind(&file.review_state)
            .bind(file.banned)
            .bind({
                #[cfg(feature = "labels")]
                { &file.labeled_by }
                #[cfg(not(feature = "labels"))]
                { &CommaSeparated::<String>::default() }
            });
        tx.execute(q).await?;

        if let Some(uid) = user_id {
            let q2 = sqlx::query("insert ignore into user_uploads(file,user_id) values(?,?)")
                .bind(&file.id)
                .bind(uid);
            tx.execute(q2).await?;
        }

        #[cfg(feature = "labels")]
        for lbl in &file.labels {
            let q3 =
                sqlx::query("insert ignore into upload_labels(file,label,model) values(?,?,?)")
                    .bind(&file.id)
                    .bind(&lbl.label)
                    .bind(&lbl.model);
            tx.execute(q3).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Determine the review state for a file based on its labels and the configured flag terms.
    ///
    /// Returns `ReviewState::LabelFlagged` if any label name contains one of the flag terms
    /// (case-insensitive substring match). Otherwise returns `ReviewState::None`.
    #[cfg(feature = "labels")]
    pub fn review_state_for_labels(labels: &[FileLabel], flag_terms: &[String]) -> ReviewState {
        if flag_terms.is_empty() {
            return ReviewState::None;
        }
        for label in labels {
            let lower = label.label.to_lowercase();
            for term in flag_terms {
                if lower.contains(&term.to_lowercase()) {
                    return ReviewState::LabelFlagged;
                }
            }
        }
        ReviewState::None
    }

    pub async fn get_file(&self, file: &Vec<u8>) -> Result<Option<FileUpload>, Error> {
        #[allow(unused_mut)]
        let mut result: Option<FileUpload> = sqlx::query_as("select * from uploads where id = ?")
            .bind(file)
            .fetch_optional(&self.pool)
            .await?;
        #[cfg(feature = "labels")]
        if let Some(ref mut f) = result {
            self.populate_labels(f).await?;
        }
        Ok(result)
    }

    pub async fn get_file_owners(&self, file: &Vec<u8>) -> Result<Vec<User>, Error> {
        sqlx::query_as(
            "select users.* from users, user_uploads \
        where users.id = user_uploads.user_id \
        and user_uploads.file = ?",
        )
        .bind(file)
        .fetch_all(&self.pool)
        .await
    }

    #[cfg(feature = "labels")]
    pub async fn get_file_labels(&self, file: &Vec<u8>) -> Result<Vec<FileLabel>, Error> {
        sqlx::query_as("select * from upload_labels where file = ?")
            .bind(file)
            .fetch_all(&self.pool)
            .await
    }

    #[cfg(feature = "labels")]
    pub async fn populate_labels(&self, file: &mut FileUpload) -> Result<(), Error> {
        file.labels = self.get_file_labels(&file.id).await?;
        Ok(())
    }

    #[cfg(feature = "labels")]
    pub async fn populate_labels_vec(&self, files: &mut Vec<FileUpload>) -> Result<(), Error> {
        for file in files.iter_mut() {
            self.populate_labels(file).await?;
        }
        Ok(())
    }

    pub async fn delete_file_owner(&self, file: &Vec<u8>, owner: u64) -> Result<(), Error> {
        sqlx::query("delete from user_uploads where file = ? and user_id = ?")
            .bind(file)
            .bind(owner)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_all_file_owner(&self, file: &Vec<u8>) -> Result<(), Error> {
        sqlx::query("delete from user_uploads where file = ?")
            .bind(file)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_file(&self, file: &Vec<u8>) -> Result<(), Error> {
        sqlx::query("delete from uploads where id = ?")
            .bind(file)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mark a file as banned: removes all ownership records and sets `banned = true`.
    /// The row is intentionally kept so re-uploads of the same hash are rejected.
    pub async fn ban_file(&self, file: &[u8]) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("delete from user_uploads where file = ?")
            .bind(file)
            .execute(&mut *tx)
            .await?;
        sqlx::query("update uploads set banned = true where id = ?")
            .bind(file)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Returns true if the file hash exists and has been banned.
    pub async fn is_file_banned(&self, file: &[u8]) -> Result<bool, Error> {
        let row = sqlx::query("select banned from uploads where id = ?")
            .bind(file)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .map(|r| r.try_get::<bool, _>(0).unwrap_or(false))
            .unwrap_or(false))
    }

    pub async fn list_files(
        &self,
        pubkey: &Vec<u8>,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<FileUpload>, i64), Error> {
        #[allow(unused_mut)]
        let mut results: Vec<FileUpload> = sqlx::query_as(
            "select uploads.* from uploads, users, user_uploads \
            where users.pubkey = ? \
            and users.id = user_uploads.user_id \
            and user_uploads.file = uploads.id \
            and uploads.banned = false \
            order by uploads.created desc \
            limit ? offset ?",
        )
        .bind(pubkey)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let count: i64 = sqlx::query(
            "select count(uploads.id) from uploads, users, user_uploads \
            where users.pubkey = ? \
            and users.id = user_uploads.user_id \
            and user_uploads.file = uploads.id",
        )
        .bind(pubkey)
        .fetch_one(&self.pool)
        .await?
        .try_get(0)?;

        #[cfg(feature = "labels")]
        self.populate_labels_vec(&mut results).await?;

        Ok((results, count))
    }

    pub async fn get_user_file_ids(&self, pubkey: &Vec<u8>) -> Result<Vec<Vec<u8>>, Error> {
        let results: Vec<(Vec<u8>,)> = sqlx::query_as(
            "select uploads.id from uploads, users, user_uploads \
            where users.pubkey = ? \
            and users.id = user_uploads.user_id \
            and user_uploads.file = uploads.id",
        )
        .bind(pubkey)
        .fetch_all(&self.pool)
        .await?;

        Ok(results.into_iter().map(|(id,)| id).collect())
    }

    /// Add a new report to the database
    pub async fn add_report(
        &self,
        file_id: &[u8],
        reporter_id: u64,
        event_json: &str,
    ) -> Result<(), Error> {
        sqlx::query("insert into reports (file_id, reporter_id, event_json) values (?, ?, ?)")
            .bind(file_id)
            .bind(reporter_id)
            .bind(event_json)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List reports with pagination for admin view
    pub async fn list_reports(&self, offset: u32, limit: u32) -> Result<(Vec<Report>, i64), Error> {
        let reports: Vec<Report> = sqlx::query_as(
            "select id, file_id, reporter_id, event_json, created, reviewed from reports where reviewed = false order by created desc limit ? offset ?"
        )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let count: i64 = sqlx::query("select count(id) from reports where reviewed = false")
            .fetch_one(&self.pool)
            .await?
            .try_get(0)?;

        Ok((reports, count))
    }

    /// Get reports for a specific file
    pub async fn get_file_reports(&self, file_id: &[u8]) -> Result<Vec<Report>, Error> {
        sqlx::query_as(
            "select id, file_id, reporter_id, event_json, created, reviewed from reports where file_id = ? order by created desc"
        )
            .bind(file_id)
            .fetch_all(&self.pool)
            .await
    }

    /// Mark a report as reviewed (used for acknowledging)
    pub async fn mark_report_reviewed(&self, report_id: u64) -> Result<(), Error> {
        sqlx::query("update reports set reviewed = true where id = ?")
            .bind(report_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Insert a single label for a file (ignores duplicates).
    #[cfg(feature = "labels")]
    pub async fn add_file_label(&self, file_id: &[u8], label: &FileLabel) -> Result<(), Error> {
        sqlx::query("insert ignore into upload_labels(file,label,model) values(?,?,?)")
            .bind(file_id)
            .bind(&label.label)
            .bind(&label.model)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Append `model_name` to the `labeled_by` column for a file, ensuring it
    /// is not added twice.
    #[cfg(feature = "labels")]
    pub async fn add_labeled_by(&self, file_id: &[u8], model_name: &str) -> Result<(), Error> {
        // Read current value, append if not already present, then write back.
        let row: Option<(CommaSeparated<String>,)> =
            sqlx::query_as("select labeled_by from uploads where id = ?")
                .bind(file_id)
                .fetch_optional(&self.pool)
                .await?;

        let mut labeled_by = row.map(|(v,)| v).unwrap_or_default();
        if !labeled_by.iter().any(|m| m == model_name) {
            labeled_by.push(model_name.to_string());
            sqlx::query("update uploads set labeled_by = ? where id = ?")
                .bind(&labeled_by)
                .bind(file_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Return image/video uploads not yet labeled by `model_name`.
    #[cfg(feature = "labels")]
    pub async fn get_files_missing_labels(
        &self,
        model_name: &str,
    ) -> Result<Vec<FileUpload>, Error> {
        sqlx::query_as(
            "select * from uploads \
             where (mime_type like 'image/%' or mime_type like 'video/%') \
             and not find_in_set(?, labeled_by)",
        )
        .bind(model_name)
        .fetch_all(&self.pool)
        .await
    }

    /// Update the review state of a file.
    pub async fn set_file_review_state(
        &self,
        file_id: &[u8],
        state: ReviewState,
    ) -> Result<(), Error> {
        sqlx::query("update uploads set review_state = ? where id = ?")
            .bind(state)
            .bind(file_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(feature = "payments")]
impl Database {
    pub async fn insert_payment(&self, payment: &Payment) -> Result<(), Error> {
        sqlx::query("insert into payments(payment_hash,user_id,amount,days_value,size_value,rate) values(?,?,?,?,?,?)")
            .bind(&payment.payment_hash)
            .bind(payment.user_id)
            .bind(payment.amount)
            .bind(payment.days_value)
            .bind(payment.size_value)
            .bind(payment.rate)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_payment(&self, payment_hash: &Vec<u8>) -> Result<Option<Payment>, Error> {
        sqlx::query_as("select * from payments where payment_hash = ?")
            .bind(payment_hash)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_user_payments(&self, uid: u64) -> Result<Vec<Payment>, Error> {
        sqlx::query_as("select * from payments where user_id = ?")
            .bind(uid)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn complete_payment(&self, payment: &Payment) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("update payments set is_paid = true, settle_index = ? where payment_hash = ?")
            .bind(payment.settle_index)
            .bind(&payment.payment_hash)
            .execute(&mut *tx)
            .await?;

        // Calculate time extension based on fractional quota value
        // If user upgrades from 5GB to 10GB, their remaining time gets halved
        // If user pays for 1GB on a 5GB plan, they get 1/5 of the normal time
        let current_user = self.get_user_by_id(payment.user_id).await?;

        if let Some(paid_until) = current_user.paid_until {
            if paid_until > chrono::Utc::now() {
                // User has active subscription - calculate fractional time extension
                let time_fraction = if current_user.paid_size > 0 {
                    payment.size_value as f64 / current_user.paid_size as f64
                } else {
                    1.0 // If no existing quota, treat as 100%
                };

                let adjusted_days = (payment.days_value as f64 * time_fraction) as u64;

                // Extend subscription time and upgrade quota if larger
                let new_quota_size = std::cmp::max(current_user.paid_size, payment.size_value);

                sqlx::query("update users set paid_until = TIMESTAMPADD(DAY, ?, paid_until), paid_size = ? where id = ?")
                    .bind(adjusted_days)
                    .bind(new_quota_size)
                    .bind(payment.user_id)
                    .execute(&mut *tx)
                    .await?;
            } else {
                // Expired subscription - set new quota and time
                sqlx::query("update users set paid_until = TIMESTAMPADD(DAY, ?, current_timestamp), paid_size = ? where id = ?")
                    .bind(payment.days_value)
                    .bind(payment.size_value)
                    .bind(payment.user_id)
                    .execute(&mut *tx)
                    .await?;
            }
        } else {
            // No existing subscription - set new quota
            sqlx::query("update users set paid_until = TIMESTAMPADD(DAY, ?, current_timestamp), paid_size = ? where id = ?")
                .bind(payment.days_value)
                .bind(payment.size_value)
                .bind(payment.user_id)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    /// Check if user has sufficient quota for an upload
    pub async fn check_user_quota(
        &self,
        pubkey: &Vec<u8>,
        upload_size: u64,
        free_quota_bytes: u64,
    ) -> Result<bool, Error> {
        // Get or create user
        let user_id = self.upsert_user(pubkey).await?;

        // Get user's current storage usage
        let user_stats = self.get_user_stats(user_id).await.unwrap_or(UserStats {
            file_count: 0,
            total_size: 0,
        });

        // Get user's paid quota
        let user = self.get_user(pubkey).await?;
        let (paid_size, paid_until) = (user.paid_size, user.paid_until);

        // Calculate total available quota
        let mut available_quota = free_quota_bytes;

        // Add paid quota if still valid
        if let Some(paid_until) = paid_until {
            if paid_until > chrono::Utc::now() {
                available_quota += paid_size;
            }
        }

        // Check if upload would exceed quota
        Ok(user_stats.total_size + upload_size <= available_quota)
    }
}

#[cfg(all(test, feature = "labels"))]
mod tests {
    use super::*;

    fn make_label(label: &str) -> FileLabel {
        FileLabel::new(label.to_string(), "vit224".to_string())
    }

    #[test]
    fn test_review_state_no_labels_no_terms() {
        let state = Database::review_state_for_labels(&[], &[]);
        assert_eq!(state, ReviewState::None);
    }

    #[test]
    fn test_review_state_labels_but_no_terms() {
        let labels = vec![make_label("cat"), make_label("dog")];
        let state = Database::review_state_for_labels(&labels, &[]);
        assert_eq!(state, ReviewState::None);
    }

    #[test]
    fn test_review_state_no_labels_with_terms() {
        let terms = vec!["nsfw".to_string(), "violence".to_string()];
        let state = Database::review_state_for_labels(&[], &terms);
        assert_eq!(state, ReviewState::None);
    }

    #[test]
    fn test_review_state_exact_match() {
        let labels = vec![make_label("nsfw")];
        let terms = vec!["nsfw".to_string()];
        let state = Database::review_state_for_labels(&labels, &terms);
        assert_eq!(state, ReviewState::LabelFlagged);
    }

    #[test]
    fn test_review_state_substring_match() {
        let labels = vec![make_label("explicit_nsfw_content")];
        let terms = vec!["nsfw".to_string()];
        let state = Database::review_state_for_labels(&labels, &terms);
        assert_eq!(state, ReviewState::LabelFlagged);
    }

    #[test]
    fn test_review_state_case_insensitive_label() {
        let labels = vec![make_label("NSFW")];
        let terms = vec!["nsfw".to_string()];
        let state = Database::review_state_for_labels(&labels, &terms);
        assert_eq!(state, ReviewState::LabelFlagged);
    }

    #[test]
    fn test_review_state_case_insensitive_term() {
        let labels = vec![make_label("nsfw")];
        let terms = vec!["NSFW".to_string()];
        let state = Database::review_state_for_labels(&labels, &terms);
        assert_eq!(state, ReviewState::LabelFlagged);
    }

    #[test]
    fn test_review_state_no_match() {
        let labels = vec![make_label("cat"), make_label("landscape")];
        let terms = vec!["nsfw".to_string(), "violence".to_string()];
        let state = Database::review_state_for_labels(&labels, &terms);
        assert_eq!(state, ReviewState::None);
    }

    #[test]
    fn test_review_state_multiple_labels_one_matches() {
        let labels = vec![
            make_label("cat"),
            make_label("nudity"),
            make_label("landscape"),
        ];
        let terms = vec!["nudity".to_string()];
        let state = Database::review_state_for_labels(&labels, &terms);
        assert_eq!(state, ReviewState::LabelFlagged);
    }

    #[test]
    fn test_review_state_default_is_none() {
        assert_eq!(ReviewState::default(), ReviewState::None);
    }
}
