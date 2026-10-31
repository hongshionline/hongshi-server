-- =============================================================================
--  hongshi-server 数据库初始化脚本
--  依据文档：doc/数据库-User表.md、doc/数据库-Friend表.md
--  运行环境：MySQL 8.0.16+（InnoDB / utf8mb4）
--  使用方式：mysql -u root -p --default-character-set=utf8mb4 < 20261031000000_init.sql
--
--  说明：
--    1. 幂等脚本：可重复执行，不会删除或覆盖已有数据。
--    2. 库名 hongshionline 与 src/db/sql.rs 中的 DB_NAME 常量一致。
--    3. uuid 由应用层生成，以 36 位标准 UUID 文本（含连字符）存储。
--    4. 所有时间戳统一为 BIGINT 毫秒级 Unix 时间戳（对应 Rust i64）。
--    5. User 表不建 friends 列：文档中的好友列表由 friend 表表达，
--       friend 表一行即代表一对好友（双方 uuid）。
-- =============================================================================

SET NAMES utf8mb4;

CREATE DATABASE IF NOT EXISTS `hongshionline`
    DEFAULT CHARACTER SET utf8mb4
    DEFAULT COLLATE utf8mb4_0900_ai_ci;

USE `hongshionline`;

-- -----------------------------------------------------------------------------
-- user：用户表
--   uuid           用户唯一凭证（主键）
--   username       用户名，可重复
--   password       密码哈希（长度兼容 bcrypt / argon2）
--   apikey         API Key（登录凭证，唯一）
--   email          邮箱，唯一索引
--   last_sign_time 上次上线时间戳（毫秒）
--   status         0=离线，1=在线
--   verify         验证等级，默认 1
--   extra          备用扩展字段（JSON）
-- 注：User 表的 friends 由 friend 表表示，本表不设该列。
-- -----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS `user` (
    `uuid`           CHAR(36)     NOT NULL               COMMENT '用户唯一凭证（UUID）',
    `username`       VARCHAR(64)  NOT NULL               COMMENT '用户名，可重复',
    `password`       VARCHAR(255) NOT NULL               COMMENT '密码（哈希）',
    `apikey`         VARCHAR(255) NOT NULL               COMMENT 'API Key（登录凭证，唯一）',
    `email`          VARCHAR(255) NOT NULL               COMMENT '邮箱，唯一',
    `last_sign_time` BIGINT       NOT NULL DEFAULT 0     COMMENT '上次上线时间戳（毫秒）',
    `status`         INT          NOT NULL DEFAULT 0     COMMENT '状态：0=离线，1=在线',
    `verify`         INT          NOT NULL DEFAULT 1     COMMENT '验证等级，默认 1',
    `extra`          JSON         NULL                   COMMENT '备用扩展字段',
    PRIMARY KEY (`uuid`),
    UNIQUE KEY `uk_user_email` (`email`),
    UNIQUE KEY `uk_user_apikey` (`apikey`),
    KEY `idx_user_username` (`username`)
) ENGINE = InnoDB
  DEFAULT CHARSET = utf8mb4
  COLLATE = utf8mb4_0900_ai_ci
  COMMENT = '用户表';

-- -----------------------------------------------------------------------------
-- friend：好友关系表
--   left_uuid   A 用户 uuid，外键指向 user.uuid
--   right_uuid  B 用户 uuid，外键指向 user.uuid
--   create_time 创建时间戳（毫秒）
--   extra       备用扩展字段（JSON）
-- 约束：
--   PRIMARY KEY (left_uuid, right_uuid) 防止同一对关系重复写入；
--   外键级联：用户被删除时，其好友关系自动删除；
--   好友关系无方向，建议应用层统一按 uuid 排序后写入（left < right），
--   避免 (A,B) 与 (B,A) 两条记录重复。
-- 注意：MySQL 8 不允许 CHECK 约束与带级联动作的外键共用同一列，
--   因此"禁止加自己为好友"（left_uuid <> right_uuid）由应用层校验。
-- -----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS `friend` (
    `left_uuid`   CHAR(36) NOT NULL              COMMENT 'A 用户 uuid',
    `right_uuid`  CHAR(36) NOT NULL              COMMENT 'B 用户 uuid',
    `create_time` BIGINT   NOT NULL DEFAULT 0    COMMENT '创建时间戳（毫秒）',
    `extra`       JSON     NULL                  COMMENT '备用扩展字段',
    PRIMARY KEY (`left_uuid`, `right_uuid`),
    KEY `idx_friend_right_uuid` (`right_uuid`),
    CONSTRAINT `fk_friend_left_uuid`
        FOREIGN KEY (`left_uuid`) REFERENCES `user` (`uuid`)
        ON DELETE CASCADE ON UPDATE CASCADE,
    CONSTRAINT `fk_friend_right_uuid`
        FOREIGN KEY (`right_uuid`) REFERENCES `user` (`uuid`)
        ON DELETE CASCADE ON UPDATE CASCADE
) ENGINE = InnoDB
  DEFAULT CHARSET = utf8mb4
  COLLATE = utf8mb4_0900_ai_ci
  COMMENT = '好友关系表';
