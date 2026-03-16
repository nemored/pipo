package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/nemored/pipo/internal/config"
)

func main() {
	var in string
	flag.StringVar(&in, "in", "", "path to the config file to migrate")
	flag.Parse()

	if in == "" {
		fmt.Fprintln(os.Stderr, "usage: pipo-config-migrate --in path-to-config.json")
		os.Exit(2)
	}

	backup, err := migrateConfig(in)
	if err != nil {
		fmt.Fprintf(os.Stderr, "migration failed: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Migration complete. Backup written to %s\n", backup)
}

func migrateConfig(path string) (string, error) {
	original, err := os.ReadFile(path)
	if err != nil {
		return "", fmt.Errorf("read config: %w", err)
	}

	cfg, err := config.Load(path)
	if err != nil {
		return "", fmt.Errorf("validate current schema: %w", err)
	}

	backup := fmt.Sprintf("%s.bak.%s", path, time.Now().UTC().Format("20060102T150405Z"))
	if err := os.WriteFile(backup, original, 0o600); err != nil {
		return "", fmt.Errorf("write backup: %w", err)
	}

	normalized, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return "", fmt.Errorf("marshal normalized config: %w", err)
	}
	normalized = append(normalized, '\n')
	if err := os.WriteFile(path, normalized, 0o600); err != nil {
		return "", fmt.Errorf("write migrated config: %w", err)
	}

	if _, err := os.Stat(backup); err != nil {
		return "", errors.New("backup file missing after migration")
	}
	return filepath.Clean(backup), nil
}
