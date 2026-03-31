-- Dynamic label models configuration table.
-- Each row represents a configured label model.
-- The config column stores the full model configuration as JSON.
CREATE TABLE IF NOT EXISTS label_models (
    `name`      VARCHAR(255) NOT NULL,
    `type`      VARCHAR(50)  NOT NULL,
    `config`    TEXT         NOT NULL,
    `created`   DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    `updated`   DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    PRIMARY KEY (`name`)
);
