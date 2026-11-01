-- =============================================================================
--  user.have_home：记录用户当前拥有的房间（房间 uuid）
--
--  - NULL     表示该用户当前没有房间
--  - 非 NULL  表示该用户已拥有此 uuid 的房间（一个用户最多一个房间）
--  - 房间列表本身不入库，这里只保存"用户 -> 房间 uuid"的归属关系
--
--  说明：
--    1. 使用独立迁移文件，而不是修改 20261031000000_init.sql，
--       避免 sqlx 对已应用迁移做 checksum 校验时报错。
--    2. have_home 允许为 NULL（MySQL 唯一索引允许多个 NULL），
--       因此"未拥有房间"的用户不会互相冲突。
-- =============================================================================

ALTER TABLE `user`
    ADD COLUMN `have_home` CHAR(36) NULL DEFAULT NULL
        COMMENT '拥有的房间UUID，NULL表示未拥有' AFTER `verify`,
    ADD UNIQUE KEY `uk_user_have_home` (`have_home`);
