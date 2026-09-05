-- A proxy endpoint has one stable node identity across manual and tunnel
-- registrations. MySQL 8 performs ALTER TABLE atomically; if historical
-- duplicates exist this migration fails without choosing or deleting a row.
-- Diagnose with:
-- SELECT ip, port, COUNT(*)
-- FROM proxy_nodes
-- GROUP BY ip, port
-- HAVING COUNT(*) > 1;
ALTER TABLE proxy_nodes
    ADD UNIQUE INDEX uq_proxy_node_ip_port (ip, port);
