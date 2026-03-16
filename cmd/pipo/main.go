package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/store"
	"github.com/nemored/pipo/internal/transports"
)

func main() {
	if err := run(os.Args, os.Getenv); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run(args []string, getenv func(string) string) error {
	if len(args) > 1 && args[1] == "migrate-db" {
		dbPath := ""
		if len(args) > 2 {
			dbPath = args[2]
		}
		if dbPath == "" {
			dbPath = getenv("DB_PATH")
		}
		if dbPath == "" {
			fmt.Printf("Usage: %s migrate-db [path-to-db.sqlite3]\n", binaryName(args))
			return nil
		}

		s, err := store.OpenSQLite(context.Background(), dbPath)
		if err != nil {
			return err
		}
		defer s.Close()
		return s.Migrate(context.Background())
	}

	configPath, dbPath := resolvePaths(args, getenv)
	if configPath == "" || dbPath == "" {
		fmt.Printf("Usage: %s path-to-config.json [path-to-db.sqlite3]\n", binaryName(args))
		return nil
	}

	s, err := store.OpenSQLite(context.Background(), dbPath)
	if err != nil {
		return err
	}
	defer s.Close()

	cfg, err := config.Load(configPath)
	if err != nil {
		return err
	}

	transportRunners, err := transports.Build(cfg)
	if err != nil {
		return err
	}

	busIDs := make([]string, 0, len(cfg.Buses))
	for _, bus := range cfg.Buses {
		busIDs = append(busIDs, bus.ID)
	}

	runtime := core.NewRuntime(busIDs, transportRunners)
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	return runtime.Run(ctx)
}

func binaryName(args []string) string {
	binaryName := "pipo"
	if len(args) > 0 && args[0] != "" {
		binaryName = args[0]
	}
	return binaryName
}

func resolvePaths(args []string, getenv func(string) string) (string, string) {
	var configPath string
	if len(args) > 1 {
		configPath = args[1]
	}
	if configPath == "" {
		configPath = getenv("CONFIG_PATH")
	}

	var dbPath string
	if len(args) > 2 {
		dbPath = args[2]
	}
	if dbPath == "" {
		dbPath = getenv("DB_PATH")
	}

	return configPath, dbPath
}
