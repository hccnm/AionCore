ALTER TABLE audit_logs
    DROP CONSTRAINT IF EXISTS audit_logs_actor_user_id_fkey;

ALTER TABLE audit_logs
    ADD CONSTRAINT audit_logs_actor_user_id_fkey
    FOREIGN KEY (actor_user_id)
    REFERENCES users(id)
    ON DELETE SET NULL;
