package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
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

func OpenSQLite(ctx context.Context, path string) (*SQLiteStore, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, err
	}
	if err := db.PingContext(ctx); err != nil {
		_ = db.Close()
		return nil, err
	}

	if err := bootstrapMessagesSchema(ctx, db); err != nil {
		_ = db.Close()
		return nil, err
	}

	nextID, err := seedNextID(ctx, db)
	if err != nil {
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
	return err
}

func seedNextID(ctx context.Context, db *sql.DB) (int64, error) {
	const q = `SELECT id FROM messages ORDER BY modtime DESC`
	var id int64
	err := db.QueryRowContext(ctx, q).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		id = 0
	} else if err != nil {
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
		return 0, err
	}
	return ret, nil
}

func (s *SQLiteStore) InsertOrReplaceSlack(ctx context.Context, slackID string) (int64, error) {
	id := s.allocID()
	const q = `INSERT OR REPLACE INTO messages (id, slackid) VALUES (?1, ?2)`
	if _, err := s.db.ExecContext(ctx, q, id, slackID); err != nil {
		return 0, err
	}
	return id, nil
}

func (s *SQLiteStore) UpdateSlackByID(ctx context.Context, pipoID int64, slackID string) error {
	const q = `UPDATE messages SET slackid = ?2 WHERE id = ?1`
	_, err := s.db.ExecContext(ctx, q, pipoID, slackID)
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
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) SelectSlackByID(ctx context.Context, pipoID int64) (*string, error) {
	const q = `SELECT slackid FROM messages WHERE id = ?1`
	var id string
	err := s.db.QueryRowContext(ctx, q, pipoID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) SelectSlackByDiscord(ctx context.Context, discordID uint64) (*string, error) {
	const q = `SELECT slackid FROM messages WHERE discordid = ?1`
	var id string
	err := s.db.QueryRowContext(ctx, q, discordID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) SelectDiscordBySlack(ctx context.Context, slackID string) (*uint64, error) {
	const q = `SELECT discordid FROM messages WHERE slackid = ?1`
	var id uint64
	err := s.db.QueryRowContext(ctx, q, slackID).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &id, nil
}

func (s *SQLiteStore) InsertOrReplaceDiscord(ctx context.Context, discordID uint64) (int64, error) {
	id := s.allocID()
	const q = `INSERT OR REPLACE INTO messages (id, discordid) VALUES (?1, ?2)`
	if _, err := s.db.ExecContext(ctx, q, id, discordID); err != nil {
		return 0, err
	}
	return id, nil
}

func (s *SQLiteStore) UpdateDiscordByID(ctx context.Context, pipoID int64, discordID uint64) error {
	const q = `UPDATE messages SET discordid = ?2 WHERE id = ?1`
	_, err := s.db.ExecContext(ctx, q, pipoID, discordID)
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
		return err
	}
	return nil
}

func (s *SQLiteStore) DebugCount(ctx context.Context) (int64, error) {
	var c int64
	if err := s.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM messages`).Scan(&c); err != nil {
		return 0, fmt.Errorf("count messages: %w", err)
	}
	return c, nil
}
