package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"sync"

	_ "modernc.org/sqlite"
)

const maxPipoID int64 = 40000

// SQLiteStore mirrors Rust messages-table semantics for ID allocation and
// Slack/Discord ID mapping.
type SQLiteStore struct {
	db     *sql.DB
	mu     sync.Mutex
	nextID int64
}

type MessageRow struct {
	ID        int64   `json:"id"`
	SlackID   *string `json:"slack_id,omitempty"`
	DiscordID *uint64 `json:"discord_id,omitempty"`
}

func OpenSQLite(ctx context.Context, path string) (*SQLiteStore, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		observeDBError("open", err)
		return nil, err
	}
	if err := db.PingContext(ctx); err != nil {
		observeDBError("ping", err)
		_ = db.Close()
		return nil, err
	}

	if err := bootstrapMessagesSchema(ctx, db); err != nil {
		observeDBError("bootstrap_schema", err)
		_ = db.Close()
		return nil, err
	}

	nextID, err := seedNextID(ctx, db)
	if err != nil {
		observeDBError("seed_next_id", err)
		_ = db.Close()
		return nil, err
	}

	return &SQLiteStore{db: db, nextID: nextID}, nil
}

func (s *SQLiteStore) Close() error {
	return s.db.Close()
}

func bootstrapMessagesSchema(ctx context.Context, db *sql.DB) error {
	const tableExists = `SELECT name FROM sqlite_master WHERE type='table' AND name='messages'`
	var name string
	err := db.QueryRowContext(ctx, tableExists).Scan(&name)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		observeDBError("schema_table_exists", err)
		return err
	}
	if err == nil {
		return nil
	}

	const schema = `CREATE TABLE messages (
		id        INTEGER PRIMARY KEY,
		slackid   TEXT,
		discordid INTEGER,
		modtime   DEFAULT (strftime('%Y-%m-%d %H:%M:%S:%s', 'now', 'localtime'))
	);
	CREATE TRIGGER updatemodtime
	BEFORE update ON messages
	begin
		update messages
		   set modtime = strftime('%Y-%m-%d %H:%M:%S:%s', 'now', 'localtime')
		 where id = old.id;
	end;`
	_, err = db.ExecContext(ctx, schema)
	observeDBError("schema_exec", err)
	return err
}

func bootstrapIRCIdentitySchema(ctx context.Context, db *sql.DB) error {
	const schema = `CREATE TABLE IF NOT EXISTS irc_message_identity (
		source_key    TEXT PRIMARY KEY,
		pipo_id       INTEGER NOT NULL,
		network       TEXT NOT NULL,
		channel       TEXT NOT NULL,
		sender        TEXT NOT NULL,
		message_token TEXT,
		message_time  TEXT,
		message_raw   TEXT,
		created_at    DEFAULT (strftime('%Y-%m-%d %H:%M:%S:%s', 'now', 'localtime'))
	);
	CREATE INDEX IF NOT EXISTS idx_irc_identity_token ON irc_message_identity(network, channel, message_token);
	CREATE INDEX IF NOT EXISTS idx_irc_identity_pipo_id ON irc_message_identity(pipo_id);`
	_, err := db.ExecContext(ctx, schema)
	observeDBError("schema_irc_identity_exec", err)
	return err
}

func seedNextID(ctx context.Context, db *sql.DB) (int64, error) {
	const q = `SELECT id FROM messages ORDER BY modtime DESC`
	var id int64
	err := db.QueryRowContext(ctx, q).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		id = 0
	} else if err != nil {
		observeDBError("seed_query", err)
		return 0, err
	}
	return id + 1, nil
}

func (s *SQLiteStore) allocID() int64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	ret := s.nextID
	s.nextID++
	if s.nextID > maxPipoID {
		s.nextID = 0
	}
	return ret
}

// InsertAllocatedID mirrors IRC/Mumble/Rachni allocation paths.
func (s *SQLiteStore) InsertAllocatedID(ctx context.Context) (int64, error) {
	id := s.allocID()
	const q = `INSERT OR REPLACE INTO messages (id) VALUES (?1)`
	if _, err := s.db.ExecContext(ctx, q, id); err != nil {
		observeDBError("insert_allocated_id", err)
		return 0, err
	}
	return id, nil
}

// InsertAllocatedIDRachni preserves Rust's off-by-one return behavior for
// Rachni: insert current id, increment shared counter, then return incremented value.
func (s *SQLiteStore) InsertAllocatedIDRachni(ctx context.Context) (int64, error) {
	s.mu.Lock()
	id := s.nextID
	s.nextID++
	if s.nextID > maxPipoID {
		s.nextID = 0
	}
	ret := s.nextID
	s.mu.Unlock()

	const q = `INSERT OR REPLACE INTO messages (id) VALUES (?1)`
	if _, err := s.db.ExecContext(ctx, q, id); err != nil {
		observeDBError("insert_allocated_id_rachni", err)
		return 0, err
	}
	return ret, nil
}

func (s *SQLiteStore) InsertOrReplaceSlack(ctx context.Context, slackID string) (int64, error) {
	id := s.allocID()
	const q = `INSERT OR REPLACE INTO messages (id, slackid) VALUES (?1, ?2)`
	if _, err := s.db.ExecContext(ctx, q, id, slackID); err != nil {
		observeDBError("insert_or_replace_slack", err)
		return 0, err
	}
	return id, nil
}

func (s *SQLiteStore) UpdateSlackByID(ctx context.Context, pipoID int64, slackID string) error {
	const q = `UPDATE messages SET slackid = ?2 WHERE id = ?1`
	_, err := s.db.ExecContext(ctx, q, pipoID, slackID)
	observeDBError("update_slack_by_id", err)
	return err
}

func (s *SQLiteStore) SelectIDBySlack(ctx context.Context, slackID string) (*int64, error) {
	const q = `SELECT id FROM messages WHERE slackid = ?1`
	var id int64
	err := s.db.QueryRowContext(ctx, q, slackID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		observeDBError("select_id_by_slack", err)
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) SelectSlackByID(ctx context.Context, pipoID int64) (*string, error) {
	const q = `SELECT slackid FROM messages WHERE id = ?1`
	var id sql.NullString
	err := s.db.QueryRowContext(ctx, q, pipoID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		observeDBError("select_slack_by_id", err)
		return nil, err
	}
	if !id.Valid {
		return nil, nil
	}
	return &id.String, nil
}

func (s *SQLiteStore) SelectSlackByDiscord(ctx context.Context, discordID uint64) (*string, error) {
	const q = `SELECT slackid FROM messages WHERE discordid = ?1`
	var id sql.NullString
	err := s.db.QueryRowContext(ctx, q, discordID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		observeDBError("select_slack_by_discord", err)
		return nil, err
	}
	if !id.Valid {
		return nil, nil
	}
	return &id.String, nil
}

func (s *SQLiteStore) SelectDiscordBySlack(ctx context.Context, slackID string) (*uint64, error) {
	const q = `SELECT discordid FROM messages WHERE slackid = ?1`
	var id uint64
	err := s.db.QueryRowContext(ctx, q, slackID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		observeDBError("select_discord_by_slack", err)
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) InsertOrReplaceDiscord(ctx context.Context, discordID uint64) (int64, error) {
	id := s.allocID()
	const q = `INSERT OR REPLACE INTO messages (id, discordid) VALUES (?1, ?2)`
	if _, err := s.db.ExecContext(ctx, q, id, discordID); err != nil {
		observeDBError("insert_or_replace_discord", err)
		return 0, err
	}
	return id, nil
}

func (s *SQLiteStore) UpdateDiscordByID(ctx context.Context, pipoID int64, discordID uint64) error {
	const q = `UPDATE messages SET discordid = ?2 WHERE id = ?1`
	_, err := s.db.ExecContext(ctx, q, pipoID, discordID)
	observeDBError("update_discord_by_id", err)
	return err
}

func (s *SQLiteStore) SelectIDByDiscord(ctx context.Context, discordID uint64) (*int64, error) {
	const q = `SELECT id FROM messages WHERE discordid = ?1`
	var id int64
	err := s.db.QueryRowContext(ctx, q, discordID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		observeDBError("select_id_by_discord", err)
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) SelectDiscordByID(ctx context.Context, pipoID int64) (*uint64, error) {
	const q = `SELECT discordid FROM messages WHERE id = ?1`
	var id uint64
	err := s.db.QueryRowContext(ctx, q, pipoID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		observeDBError("select_discord_by_id", err)
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) Migrate(ctx context.Context) error {
	// Current schema is already compatible. This command is intentionally
	// idempotent for future in-place migrations.
	const createTable = `CREATE TABLE IF NOT EXISTS messages (
		id        INTEGER PRIMARY KEY,
		slackid   TEXT,
		discordid INTEGER,
		modtime   DEFAULT (strftime('%Y-%m-%d %H:%M:%S:%s', 'now', 'localtime'))
	)`
	if _, err := s.db.ExecContext(ctx, createTable); err != nil {
		observeDBError("migrate_create_table", err)
		return err
	}
	const createTrigger = `CREATE TRIGGER IF NOT EXISTS updatemodtime
	BEFORE update ON messages
	begin
		update messages
		   set modtime = strftime('%Y-%m-%d %H:%M:%S:%s', 'now', 'localtime')
		 where id = old.id;
	end;`
	if _, err := s.db.ExecContext(ctx, createTrigger); err != nil {
		observeDBError("migrate_create_trigger", err)
		return err
	}
	return nil
}

type IRCIdentityRecord struct {
	SourceKey    string
	Network      string
	Channel      string
	Sender       string
	MessageToken *string
	MessageTime  *string
	MessageRaw   *string
}

func (s *SQLiteStore) ensureIRCIdentitySchema(ctx context.Context) error {
	err := bootstrapIRCIdentitySchema(ctx, s.db)
	if err != nil {
		observeDBError("ensure_irc_identity_schema", err)
	}
	return err
}

func (s *SQLiteStore) EnsureIRCIdentity(ctx context.Context, rec IRCIdentityRecord) (int64, bool, error) {
	if err := s.ensureIRCIdentitySchema(ctx); err != nil {
		return 0, false, err
	}
	if strings.TrimSpace(rec.SourceKey) == "" {
		return 0, false, fmt.Errorf("irc source key is required")
	}
	if id, err := s.SelectPipoIDByIRCSourceKey(ctx, rec.SourceKey); err != nil {
		observeDBError("select_irc_identity_by_source_key", err)
		return 0, false, err
	} else if id != nil {
		return *id, false, nil
	}

	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		observeDBError("begin_ensure_irc_identity", err)
		return 0, false, err
	}
	defer func() { _ = tx.Rollback() }()

	var existing int64
	err = tx.QueryRowContext(ctx, `SELECT pipo_id FROM irc_message_identity WHERE source_key = ?1`, rec.SourceKey).Scan(&existing)
	if err == nil {
		if err := tx.Commit(); err != nil {
			observeDBError("commit_ensure_irc_identity_existing", err)
			return 0, false, err
		}
		return existing, false, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		observeDBError("query_ensure_irc_identity_existing", err)
		return 0, false, err
	}

	id := s.allocID()
	if _, err := tx.ExecContext(ctx, `INSERT OR REPLACE INTO messages (id) VALUES (?1)`, id); err != nil {
		observeDBError("insert_messages_ensure_irc_identity", err)
		return 0, false, err
	}
	if _, err := tx.ExecContext(ctx, `INSERT INTO irc_message_identity
		(source_key, pipo_id, network, channel, sender, message_token, message_time, message_raw)
		VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)`,
		rec.SourceKey,
		id,
		rec.Network,
		rec.Channel,
		rec.Sender,
		rec.MessageToken,
		rec.MessageTime,
		rec.MessageRaw,
	); err != nil {
		observeDBError("insert_irc_identity", err)
		if existingID, lookupErr := s.SelectPipoIDByIRCSourceKey(ctx, rec.SourceKey); lookupErr == nil && existingID != nil {
			return *existingID, false, nil
		}
		return 0, false, err
	}
	if err := tx.Commit(); err != nil {
		observeDBError("commit_ensure_irc_identity", err)
		return 0, false, err
	}
	return id, true, nil
}

func (s *SQLiteStore) SelectPipoIDByIRCSourceKey(ctx context.Context, sourceKey string) (*int64, error) {
	if err := s.ensureIRCIdentitySchema(ctx); err != nil {
		return nil, err
	}
	const q = `SELECT pipo_id FROM irc_message_identity WHERE source_key = ?1`
	var id int64
	err := s.db.QueryRowContext(ctx, q, sourceKey).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		observeDBError("select_pipo_id_by_irc_source_key", err)
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) SelectPipoIDByIRCToken(ctx context.Context, network, channel, token string) (*int64, error) {
	if err := s.ensureIRCIdentitySchema(ctx); err != nil {
		return nil, err
	}
	const q = `SELECT pipo_id FROM irc_message_identity WHERE network = ?1 AND channel = ?2 AND message_token = ?3 ORDER BY created_at DESC LIMIT 1`
	var id int64
	err := s.db.QueryRowContext(ctx, q, network, channel, token).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		observeDBError("select_pipo_id_by_irc_token", err)
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) DebugCount(ctx context.Context) (int64, error) {
	var c int64
	if err := s.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM messages`).Scan(&c); err != nil {
		observeDBError("debug_count", err)
		return 0, fmt.Errorf("count messages: %w", err)
	}
	return c, nil
}

func (s *SQLiteStore) DebugRows(ctx context.Context) ([]MessageRow, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT id, slackid, discordid FROM messages ORDER BY id`)
	if err != nil {
		observeDBError("debug_rows_query", err)
		return nil, fmt.Errorf("query rows: %w", err)
	}
	defer rows.Close()

	out := make([]MessageRow, 0)
	for rows.Next() {
		var (
			id         int64
			slackRaw   sql.NullString
			discordRaw sql.NullInt64
		)
		if err := rows.Scan(&id, &slackRaw, &discordRaw); err != nil {
			observeDBError("debug_rows_scan", err)
			return nil, fmt.Errorf("scan row: %w", err)
		}
		var slack *string
		if slackRaw.Valid {
			v := slackRaw.String
			slack = &v
		}
		var discord *uint64
		if discordRaw.Valid {
			v := uint64(discordRaw.Int64)
			discord = &v
		}
		out = append(out, MessageRow{ID: id, SlackID: slack, DiscordID: discord})
	}
	if err := rows.Err(); err != nil {
		observeDBError("debug_rows_iterate", err)
		return nil, fmt.Errorf("iterate rows: %w", err)
	}
	return out, nil
}
