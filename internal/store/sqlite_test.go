package store

import (
	"context"
	"database/sql"
	"testing"
	"time"
)

func TestBootstrapAndMappingQueries(t *testing.T) {
	ctx := context.Background()
	s, err := OpenSQLite(ctx, ":memory:")
	if err != nil {
		t.Fatalf("open sqlite: %v", err)
	}
	defer s.Close()

	id, err := s.InsertOrReplaceSlack(ctx, "1700000000.100")
	if err != nil {
		t.Fatalf("insert slack: %v", err)
	}
	if id != 1 {
		t.Fatalf("expected first allocated id to be 1, got %d", id)
	}

	gotID, err := s.SelectIDBySlack(ctx, "1700000000.100")
	if err != nil || gotID == nil || *gotID != id {
		t.Fatalf("select id by slack mismatch: id=%v err=%v", gotID, err)
	}

	if err := s.UpdateDiscordByID(ctx, id, 42); err != nil {
		t.Fatalf("update discord: %v", err)
	}

	discordID, err := s.SelectDiscordByID(ctx, id)
	if err != nil || discordID == nil || *discordID != 42 {
		t.Fatalf("select discord by id mismatch: id=%v err=%v", discordID, err)
	}

	slackID, err := s.SelectSlackByDiscord(ctx, 42)
	if err != nil || slackID == nil || *slackID != "1700000000.100" {
		t.Fatalf("cross-map slack by discord mismatch: id=%v err=%v", slackID, err)
	}
}

func TestUpdateModtimeTrigger(t *testing.T) {
	ctx := context.Background()
	s, err := OpenSQLite(ctx, ":memory:")
	if err != nil {
		t.Fatalf("open sqlite: %v", err)
	}
	defer s.Close()

	id, err := s.InsertOrReplaceSlack(ctx, "1700000000.100")
	if err != nil {
		t.Fatalf("insert slack: %v", err)
	}

	var before string
	if err := s.db.QueryRowContext(ctx, `SELECT modtime FROM messages WHERE id = ?1`, id).Scan(&before); err != nil {
		t.Fatalf("query before modtime: %v", err)
	}

	time.Sleep(1100 * time.Millisecond)
	if err := s.UpdateSlackByID(ctx, id, "1700000000.101"); err != nil {
		t.Fatalf("update slack: %v", err)
	}

	var after string
	if err := s.db.QueryRowContext(ctx, `SELECT modtime FROM messages WHERE id = ?1`, id).Scan(&after); err != nil {
		t.Fatalf("query after modtime: %v", err)
	}

	if before == after {
		t.Fatalf("expected modtime to change on update, before=%q after=%q", before, after)
	}
}

func TestSeedFromLatestModtime(t *testing.T) {
	ctx := context.Background()
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatalf("open sqlite: %v", err)
	}
	defer db.Close()

	if err := bootstrapMessagesSchema(ctx, db); err != nil {
		t.Fatalf("bootstrap schema: %v", err)
	}

	if _, err := db.ExecContext(ctx, `INSERT INTO messages (id, modtime) VALUES (10, '2000-01-01 00:00:00:0')`); err != nil {
		t.Fatalf("insert row 10: %v", err)
	}
	if _, err := db.ExecContext(ctx, `INSERT INTO messages (id, modtime) VALUES (7, '2099-01-01 00:00:00:0')`); err != nil {
		t.Fatalf("insert row 7: %v", err)
	}

	next, err := seedNextID(ctx, db)
	if err != nil {
		t.Fatalf("seed next id: %v", err)
	}
	if next != 8 {
		t.Fatalf("expected next id from latest modtime row to be 8, got %d", next)
	}
}
