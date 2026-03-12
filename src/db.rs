use crate::file_stats::{FileStatSnapshot, FileStats};
use crate::filesystem::NewFileResult;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::migrate::MigrateError;
use sqlx::{Error, Executor, FromRow, QueryBuilder, Row};

/// Column to sort files by.
#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FileStatSort {
    /// Sort by upload creation time (default).
    #[default]
    Created,
    EgressBytes,
    LastAccessed,
}

/// Sort direction.
#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

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

/// A [`FileUpload`] row joined with its [`FileStats`] row.
///
/// Used by queries that select `u.*, fs.last_accessed, fs.egress_bytes` in one
/// go so that `query_as` can deserialise both structs without manual field
/// extraction.
#[derive(Clone, FromRow)]
pub struct FileUploadWithStats {
    #[sqlx(flatten)]
    pub upload: FileUpload,
    #[sqlx(flatten)]
    pub stats: FileStats,
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
pub struct WhitelistEntry {
    pub pubkey: String,
    pub created: DateTime<Utc>,
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
        uploads(id,name,size,mime_type,blur_hash,width,height,alt,created,duration,bitrate,review_state,banned) values(?,?,?,?,?,?,?,?,?,?,?,?,?)")
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
            .bind(file.banned);
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

    /// Fetch owners for multiple files in a single query, returning a map
    /// keyed by file id.
    pub async fn get_file_owners_batch(
        &self,
        file_ids: &[&[u8]],
    ) -> Result<std::collections::HashMap<Vec<u8>, Vec<User>>, Error> {
        use std::collections::HashMap;
        if file_ids.is_empty() {
            return Ok(HashMap::new());
        }
        // We need the file id in the result to group by, so select it alongside user columns.
        let mut qb = sqlx::QueryBuilder::new(
            "select uu.file, u.* from users u \
             join user_uploads uu on u.id = uu.user_id \
             where uu.file in (",
        );
        let mut sep = qb.separated(", ");
        for id in file_ids {
            sep.push_bind(*id);
        }
        sep.push_unseparated(")");
        let rows: Vec<sqlx::mysql::MySqlRow> = qb.build().fetch_all(&self.pool).await?;
        let mut map: HashMap<Vec<u8>, Vec<User>> = HashMap::new();
        for row in rows {
            use sqlx::Row;
            let file_id: Vec<u8> = row.try_get("file")?;
            let user = User {
                id: row.try_get("id")?,
                pubkey: row.try_get("pubkey")?,
                created: row.try_get("created")?,
                is_admin: row.try_get("is_admin")?,
                #[cfg(feature = "payments")]
                paid_until: row.try_get("paid_until")?,
                #[cfg(feature = "payments")]
                paid_size: row.try_get("paid_size")?,
            };
            map.entry(file_id).or_default().push(user);
        }
        Ok(map)
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
    pub async fn populate_labels_vec(&self, files: &mut [FileUpload]) -> Result<(), Error> {
        if files.is_empty() {
            return Ok(());
        }
        let ids: Vec<&[u8]> = files.iter().map(|f| f.id.as_slice()).collect();
        let labels = self.get_file_labels_batch(&ids).await?;
        for file in files.iter_mut() {
            if let Some(fl) = labels.get(file.id.as_slice()) {
                file.labels = fl.clone();
            }
        }
        Ok(())
    }

    /// Fetch labels for multiple files in a single query, returning a map
    /// keyed by file id.
    #[cfg(feature = "labels")]
    pub async fn get_file_labels_batch(
        &self,
        file_ids: &[&[u8]],
    ) -> Result<std::collections::HashMap<Vec<u8>, Vec<FileLabel>>, Error> {
        use std::collections::HashMap;
        if file_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut qb = sqlx::QueryBuilder::new("select * from upload_labels where file in (");
        let mut sep = qb.separated(", ");
        for id in file_ids {
            sep.push_bind(*id);
        }
        sep.push_unseparated(")");
        let all_labels: Vec<FileLabel> = qb.build_query_as().fetch_all(&self.pool).await?;
        let mut map: HashMap<Vec<u8>, Vec<FileLabel>> = HashMap::new();
        for label in all_labels {
            map.entry(label.file.clone()).or_default().push(label);
        }
        Ok(map)
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
            and user_uploads.file = uploads.id \
            and uploads.banned = false",
        )
        .bind(pubkey)
        .fetch_one(&self.pool)
        .await?
        .try_get(0)?;

        #[cfg(feature = "labels")]
        self.populate_labels_vec(&mut results).await?;

        Ok((results, count))
    }

    /// List a user's own files joined with their access statistics.
    ///
    /// Returns a page of `(FileUpload, FileStats)` tuples ordered by the
    /// requested column, plus the total un-paged count.
    pub async fn list_files_with_stats(
        &self,
        pubkey: &[u8],
        offset: u32,
        limit: u32,
        mime_type: Option<String>,
        label: Option<String>,
        sort: FileStatSort,
        order: SortOrder,
    ) -> Result<(Vec<(FileUpload, FileStats)>, i64), Error> {
        let order_sql = match order {
            SortOrder::Desc => "desc",
            SortOrder::Asc => "asc",
        };
        // Use INNER JOIN on file_stats when sorting by a stats column so that
        // nulls never appear in the sort key; LEFT JOIN otherwise so that files
        // with no recorded downloads are still included.
        let (stats_join, sort_col) = match sort {
            FileStatSort::Created => (
                "left join file_stats fs on fs.file = uploads.id",
                "uploads.created",
            ),
            FileStatSort::EgressBytes => (
                "inner join file_stats fs on fs.file = uploads.id",
                "fs.egress_bytes",
            ),
            FileStatSort::LastAccessed => (
                "inner join file_stats fs on fs.file = uploads.id",
                "fs.last_accessed",
            ),
        };

        let mut q = QueryBuilder::new(
            "select uploads.*, \
             coalesce(fs.last_accessed, null) as last_accessed, \
             cast(coalesce(fs.egress_bytes, 0) as unsigned) as egress_bytes \
             from uploads \
             join users on users.pubkey = ",
        );
        q.push_bind(pubkey);
        q.push(" join user_uploads on user_uploads.user_id = users.id and user_uploads.file = uploads.id ");
        q.push(stats_join);
        q.push(" where uploads.banned = false ");
        Self::build_user_files_where(&mut q, &mime_type, &label);
        q.push(format!("order by {} {} limit ", sort_col, order_sql));
        q.push_bind(limit);
        q.push(" offset ");
        q.push_bind(offset);

        #[allow(unused_mut)]
        let mut results: Vec<FileUploadWithStats> = q.build_query_as().fetch_all(&self.pool).await?;

        let mut cq = QueryBuilder::new(
            "select count(uploads.id) from uploads \
             join users on users.pubkey = ",
        );
        cq.push_bind(pubkey);
        cq.push(
            " join user_uploads on user_uploads.user_id = users.id \
             and user_uploads.file = uploads.id \
             where uploads.banned = false ",
        );
        Self::build_user_files_where(&mut cq, &mime_type, &label);
        let count: i64 = cq.build().fetch_one(&self.pool).await?.try_get(0)?;

        #[cfg(feature = "labels")]
        {
            let mut uploads: Vec<FileUpload> = results.iter().map(|r| r.upload.clone()).collect();
            self.populate_labels_vec(&mut uploads).await?;
            for (row, upload) in results.iter_mut().zip(uploads) {
                row.upload.labels = upload.labels;
            }
        }

        Ok((
            results.into_iter().map(|r| (r.upload, r.stats)).collect(),
            count,
        ))
    }

    /// Append optional WHERE clauses for user file queries.
    fn build_user_files_where<'a>(
        qb: &mut QueryBuilder<'a, sqlx::MySql>,
        mime_type: &'a Option<String>,
        label: &'a Option<String>,
    ) {
        if let Some(m) = mime_type {
            qb.push("and uploads.mime_type like ");
            qb.push_bind(format!("%{}%", m));
            qb.push(" ");
        }
        if let Some(l) = label {
            qb.push(
                "and exists (select 1 from upload_labels ul where ul.file = uploads.id and ul.label = ",
            );
            qb.push_bind(l.clone());
            qb.push(") ");
        }
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

    /// Delete all files owned by a user and return their IDs so the caller can
    /// remove the physical files. Runs in a single transaction:
    ///   1. collect the file IDs
    ///   2. delete all user_uploads rows for this user
    ///   3. delete all uploads rows that now have no remaining owners
    pub async fn purge_user_files(&self, pubkey: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
        let mut tx = self.pool.begin().await?;

        // Collect file IDs owned by this user
        let ids: Vec<(Vec<u8>,)> = sqlx::query_as(
            "select uu.file from user_uploads uu \
             join users u on u.id = uu.user_id \
             where u.pubkey = ?",
        )
        .bind(pubkey)
        .fetch_all(&mut *tx)
        .await?;

        if ids.is_empty() {
            return Ok(vec![]);
        }

        // Remove ownership records for this user
        sqlx::query(
            "delete uu from user_uploads uu \
             join users u on u.id = uu.user_id \
             where u.pubkey = ?",
        )
        .bind(pubkey)
        .execute(&mut *tx)
        .await?;

        // Remove upload rows that have no remaining owners
        sqlx::query(
            "delete u from uploads u \
             left join user_uploads uu on uu.file = u.id \
             where uu.file is null and u.banned = false",
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(ids.into_iter().map(|(id,)| id).collect())
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

    /// Record that `model_name` has labeled `file_id`.
    #[cfg(feature = "labels")]
    pub async fn add_labeled_by(&self, file_id: &[u8], model_name: &str) -> Result<(), Error> {
        sqlx::query("insert ignore into upload_labeled_by(file, model) values(?, ?)")
            .bind(file_id)
            .bind(model_name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Return image/video uploads not yet labeled by `model_name`.
    #[cfg(feature = "labels")]
    pub async fn get_files_missing_labels(
        &self,
        model_name: &str,
    ) -> Result<Vec<FileUpload>, Error> {
        sqlx::query_as(
            "select * from uploads u \
             where (u.mime_type like 'image/%' or u.mime_type like 'video/%') \
             and not exists (select 1 from upload_labeled_by lb where lb.file = u.id and lb.model = ?) \
             limit 100",
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

    /// Set the review state for multiple files in a single query.
    pub async fn set_files_review_state(
        &self,
        file_ids: &[Vec<u8>],
        state: ReviewState,
    ) -> Result<(), Error> {
        if file_ids.is_empty() {
            return Ok(());
        }
        let mut qb = QueryBuilder::new("update uploads set review_state = ");
        qb.push_bind(state);
        qb.push(" where id in (");
        let mut sep = qb.separated(", ");
        for id in file_ids {
            sep.push_bind(id);
        }
        sep.push_unseparated(")");
        qb.build().execute(&self.pool).await?;
        Ok(())
    }

    /// Ban multiple files in a single transaction: removes all ownership
    /// records and sets `banned = true`. Returns the list of IDs that were
    /// successfully banned.
    pub async fn ban_files(&self, file_ids: &[Vec<u8>]) -> Result<(), Error> {
        if file_ids.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;

        let mut qb = QueryBuilder::new("delete from user_uploads where file in (");
        let mut sep = qb.separated(", ");
        for id in file_ids {
            sep.push_bind(id);
        }
        sep.push_unseparated(")");
        qb.build().execute(&mut *tx).await?;

        let mut qb = QueryBuilder::new("update uploads set banned = true where id in (");
        let mut sep = qb.separated(", ");
        for id in file_ids {
            sep.push_bind(id);
        }
        sep.push_unseparated(")");
        qb.build().execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(())
    }

    // ── Database-backed whitelist ───────────────────────────────────────────

    /// Add a pubkey (hex) to the database whitelist.
    /// Uses `INSERT IGNORE` so calling this for an already-present pubkey is safe.
    pub async fn whitelist_add(&self, pubkey_hex: &str) -> Result<(), Error> {
        sqlx::query("insert ignore into whitelist(pubkey) values(?)")
            .bind(pubkey_hex)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Remove a pubkey (hex) from the database whitelist.
    pub async fn whitelist_remove(&self, pubkey_hex: &str) -> Result<(), Error> {
        sqlx::query("delete from whitelist where pubkey = ?")
            .bind(pubkey_hex)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Return all entries in the database whitelist, ordered by creation time.
    pub async fn whitelist_list(&self) -> Result<Vec<WhitelistEntry>, Error> {
        sqlx::query_as("select pubkey, created from whitelist order by created asc")
            .fetch_all(&self.pool)
            .await
    }

    // ── Database-backed dynamic config ─────────────────────────────────────

    /// Return all key/value config overrides stored in the database.
    pub async fn config_list(&self) -> Result<Vec<(String, String)>, Error> {
        let rows = sqlx::query("select `key`, `value` from config order by `key` asc")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok((r.try_get(0)?, r.try_get(1)?)))
            .collect()
    }

    /// Seed a config key from the static config file — inserts only if the key
    /// does not already exist, so existing admin overrides are never clobbered.
    pub async fn config_seed(&self, key: &str, value: &str) -> Result<(), Error> {
        sqlx::query("insert ignore into config(`key`, `value`) values(?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set (upsert) a single config key.
    pub async fn config_set(&self, key: &str, value: &str) -> Result<(), Error> {
        sqlx::query(
            "insert into config(`key`, `value`) values(?, ?) \
             on duplicate key update `value` = values(`value`), `updated` = current_timestamp",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete a single config key (reverting to the static file value).
    pub async fn config_delete(&self, key: &str) -> Result<(), Error> {
        sqlx::query("delete from config where `key` = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Return true if `pubkey_hex` is present in the database whitelist.
    pub async fn whitelist_contains(&self, pubkey_hex: &str) -> Result<bool, Error> {
        let row = sqlx::query("select 1 from whitelist where pubkey = ?")
            .bind(pubkey_hex)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Return true if the database whitelist has at least one entry.
    pub async fn whitelist_is_enabled(&self) -> Result<bool, Error> {
        let row = sqlx::query("select 1 from whitelist limit 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Mark multiple reports as reviewed in a single query.
    pub async fn mark_reports_reviewed(&self, report_ids: &[u64]) -> Result<(), Error> {
        if report_ids.is_empty() {
            return Ok(());
        }
        let mut qb = QueryBuilder::new("update reports set reviewed = true where id in (");
        let mut sep = qb.separated(", ");
        for id in report_ids {
            sep.push_bind(id);
        }
        sep.push_unseparated(")");
        qb.build().execute(&self.pool).await?;
        Ok(())
    }
}

// ── Perceptual hash (pHash / LSH) ──────────────────────────────────────────

#[cfg(feature = "media-compression")]
#[derive(Clone, FromRow)]
pub struct FilePhash {
    pub file: Vec<u8>,
    pub band0: i16,
    pub band1: i16,
    pub band2: i16,
    pub band3: i16,
}

#[cfg(feature = "media-compression")]
impl FilePhash {
    /// Hamming distance between this stored hash and `query` bands.
    pub fn hamming_distance(&self, query: &[i16; 4]) -> u32 {
        (self.band0 ^ query[0]).count_ones()
            + (self.band1 ^ query[1]).count_ones()
            + (self.band2 ^ query[2]).count_ones()
            + (self.band3 ^ query[3]).count_ones()
    }
}

/// Extract four 16-bit LSH bands from an 8-byte hash.
#[cfg(feature = "media-compression")]
fn hash_bands(hash: &[u8; 8]) -> [i16; 4] {
    [
        i16::from_be_bytes([hash[0], hash[1]]),
        i16::from_be_bytes([hash[2], hash[3]]),
        i16::from_be_bytes([hash[4], hash[5]]),
        i16::from_be_bytes([hash[6], hash[7]]),
    ]
}

#[cfg(feature = "media-compression")]
impl Database {
    /// Store a perceptual hash for a file.
    /// Uses `INSERT IGNORE` so calling this twice is safe.
    pub async fn upsert_phash(&self, file_id: &[u8], hash: &[u8; 8]) -> Result<(), Error> {
        let bands = hash_bands(hash);
        sqlx::query(
            "insert ignore into upload_phash(file, band0, band1, band2, band3) \
             values(?, ?, ?, ?, ?)",
        )
        .bind(file_id)
        .bind(bands[0])
        .bind(bands[1])
        .bind(bands[2])
        .bind(bands[3])
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Return image uploads that do not yet have a perceptual hash computed.
    pub async fn get_images_missing_phash(&self) -> Result<Vec<FileUpload>, Error> {
        sqlx::query_as(
            "select u.* from uploads u \
             where u.mime_type like 'image/%' \
             and u.banned = false \
             and not exists (select 1 from upload_phash p where p.file = u.id) \
             limit 100",
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Find files whose pHash is within `max_distance` Hamming bits of `query`,
    /// using LSH band matching as a pre-filter then exact Hamming verification.
    pub async fn find_similar_images(
        &self,
        query: &[u8; 8],
        max_distance: u32,
        exclude_file: Option<&[u8]>,
    ) -> Result<Vec<(Vec<u8>, u32)>, Error> {
        let bands = hash_bands(query);

        let mut qb = sqlx::QueryBuilder::new(
            "select file, band0, band1, band2, band3 from upload_phash where (band0 = ",
        );
        qb.push_bind(bands[0]);
        qb.push(" or band1 = ");
        qb.push_bind(bands[1]);
        qb.push(" or band2 = ");
        qb.push_bind(bands[2]);
        qb.push(" or band3 = ");
        qb.push_bind(bands[3]);
        qb.push(")");

        if let Some(ex) = exclude_file {
            qb.push(" and file != ");
            qb.push_bind(ex.to_vec());
        }

        let rows: Vec<FilePhash> = qb.build_query_as().fetch_all(&self.pool).await?;

        let mut results: Vec<(Vec<u8>, u32)> = rows
            .into_iter()
            .filter_map(|row| {
                let dist = row.hamming_distance(&bands);
                if dist <= max_distance {
                    Some((row.file, dist))
                } else {
                    None
                }
            })
            .collect();

        results.sort_by_key(|(_, d)| *d);
        Ok(results)
    }

    /// Retrieve the stored pHash for a single file, if present.
    pub async fn get_phash(&self, file_id: &[u8]) -> Result<Option<[u8; 8]>, Error> {
        let row: Option<FilePhash> = sqlx::query_as(
            "select file, band0, band1, band2, band3 from upload_phash where file = ?",
        )
        .bind(file_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| {
            let b0 = r.band0.to_be_bytes();
            let b1 = r.band1.to_be_bytes();
            let b2 = r.band2.to_be_bytes();
            let b3 = r.band3.to_be_bytes();
            [b0[0], b0[1], b1[0], b1[1], b2[0], b2[1], b3[0], b3[1]]
        }))
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

// ── File access statistics ──────────────────────────────────────────────────

impl Database {
    /// Upsert a file stats snapshot into the `file_stats` table.
    ///
    /// If a row already exists for the file, `last_accessed` is updated to the
    /// maximum of the stored and incoming value, and `egress_bytes` is
    /// incremented by the snapshot's value.
    pub async fn upsert_file_stats(&self, snap: &FileStatSnapshot) -> Result<(), Error> {
        sqlx::query(
            "insert into file_stats(file, last_accessed, egress_bytes) \
             values(?, ?, ?) \
             on duplicate key update \
               last_accessed  = greatest(coalesce(last_accessed, values(last_accessed)), values(last_accessed)), \
               egress_bytes   = egress_bytes + values(egress_bytes)",
        )
        .bind(&snap.file_id)
        .bind(snap.last_accessed)
        .bind(snap.egress_bytes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch persisted stats for a single file from the `file_stats` table.
    ///
    /// Returns `None` when no row exists (file has never been accessed).
    pub async fn get_file_stats(&self, file_id: &Vec<u8>) -> Result<Option<FileStats>, Error> {
        sqlx::query_as("select last_accessed, egress_bytes from file_stats where file = ?")
            .bind(file_id)
            .fetch_optional(&self.pool)
            .await
    }

    /// Return IDs of files that have had no downloads since `cutoff`.
    ///
    /// A file qualifies when **either**:
    /// - it has never been downloaded (no row in `file_stats`), **or**
    /// - its `last_accessed` timestamp is older than `cutoff`.
    ///
    /// Files whose `created` timestamp is newer than `cutoff` are excluded so
    /// that recently uploaded files are given a grace period equal to the same
    /// window before they can be deleted.
    ///
    /// At most `limit` IDs are returned per call so callers can process work
    /// in bounded batches.
    pub async fn get_unaccessed_files(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let rows: Vec<(Vec<u8>,)> = sqlx::query_as(
            "select u.id from uploads u \
             left join file_stats fs on fs.file = u.id \
             where u.banned = false \
             and u.created < ? \
             and (fs.last_accessed is null or fs.last_accessed < ?) \
             limit ?",
        )
        .bind(cutoff)
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Return IDs of files whose `created` timestamp is older than `cutoff`,
    /// regardless of download activity (hard retention limit).
    ///
    /// Banned files are excluded — they are kept as tombstones intentionally.
    /// At most `limit` IDs are returned per call.
    pub async fn get_files_older_than(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let rows: Vec<(Vec<u8>,)> = sqlx::query_as(
            "select id from uploads \
             where banned = false \
             and created < ? \
             limit ?",
        )
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Fetch persisted stats for a batch of files.
    ///
    /// Returns a map keyed by file id.  Files with no stats row are absent
    /// from the map; callers should treat a missing entry as all-zero stats.
    pub async fn get_file_stats_batch(
        &self,
        file_ids: &[&[u8]],
    ) -> Result<std::collections::HashMap<Vec<u8>, FileStats>, Error> {
        use std::collections::HashMap;
        if file_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut qb = QueryBuilder::new(
            "select file, last_accessed, egress_bytes from file_stats where file in (",
        );
        let mut sep = qb.separated(", ");
        for id in file_ids {
            sep.push_bind(*id);
        }
        sep.push_unseparated(")");

        #[derive(sqlx::FromRow)]
        struct Row {
            file: Vec<u8>,
            last_accessed: Option<chrono::DateTime<chrono::Utc>>,
            egress_bytes: u64,
        }

        let rows: Vec<Row> = qb.build_query_as().fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.file,
                    FileStats {
                        last_accessed: r.last_accessed,
                        egress_bytes: r.egress_bytes,
                    },
                )
            })
            .collect())
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
