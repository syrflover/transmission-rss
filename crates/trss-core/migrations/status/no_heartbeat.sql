-- One `trss` process runs on an app database, kept alone by the lock it takes
-- at its start, so the board judges a stop by the collection cycles alone and
-- nothing writes the worker's heartbeat any more.

DROP TABLE worker_heartbeat;
