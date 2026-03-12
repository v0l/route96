-- Database-backed dynamic configuration layer.
-- Each row overrides the corresponding key in the static config file.
-- Keys use dot notation to match nested YAML paths (e.g. "max_upload_bytes").
CREATE TABLE IF NOT EXISTS config
(
    `key`     VARCHAR(255) NOT NULL,
    `value`   TEXT         NOT NULL,
    `created` DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    `updated` DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    PRIMARY KEY (`key`)
);
