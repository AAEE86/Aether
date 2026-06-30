-- 移除冗余索引，减少 UPDATE 写放大，避免优化器误选非主键索引：
-- ix_user_sessions_user_id (user_id) 被 user_active (user_id, revoked_at, expires_at) 以前缀覆盖
-- ix_user_sessions_client_device_id (client_device_id) 单列索引删除安全：
--   现存所有查询均以 user_id 为前导条件，由 user_device (user_id, client_device_id) 满足；
--   若未来出现仅按 client_device_id 检索的查询，需重建该索引。

DROP INDEX IF EXISTS ix_user_sessions_user_id;
DROP INDEX IF EXISTS ix_user_sessions_client_device_id;
