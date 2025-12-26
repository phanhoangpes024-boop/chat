-- TurboChat Schema (TimeWindowCompaction)
CREATE KEYSPACE IF NOT EXISTS turbochat
WITH replication = {
    'class': 'SimpleStrategy',
    'replication_factor': 3
};

USE turbochat;

-- Messages table with time-based partitioning
CREATE TABLE IF NOT EXISTS messages (
    chat_id bigint,
    bucket int,              -- YYYYMMDD for time-window compaction
    message_id bigint,
    sender_id bigint,
    content blob,
    timestamp_us bigint,
    content_crc int,
    PRIMARY KEY ((chat_id, bucket), message_id)
) WITH CLUSTERING ORDER BY (message_id ASC)
  AND compaction = {
    'class': 'TimeWindowCompactionStrategy',
    'compaction_window_unit': 'DAYS',
    'compaction_window_size': 1
  }
  AND comment = 'High-throughput chat messages with daily compaction windows';