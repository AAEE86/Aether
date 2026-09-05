-- A proxy endpoint has one stable node identity across manual and tunnel
-- registrations. Index creation intentionally fails if historical duplicates
-- exist; operators must resolve the conflicting identities explicitly.
-- Diagnose with:
-- SELECT ip, port, COUNT(*)
-- FROM proxy_nodes
-- GROUP BY ip, port
-- HAVING COUNT(*) > 1;
CREATE UNIQUE INDEX uq_proxy_node_ip_port ON proxy_nodes (ip, port);
