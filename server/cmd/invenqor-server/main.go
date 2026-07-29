package main

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/hkjang/invenqor/server/internal/agents"
	"github.com/hkjang/invenqor/server/internal/apikeys"
	"github.com/hkjang/invenqor/server/internal/auth"
	"github.com/hkjang/invenqor/server/internal/bootstrap"
	"github.com/hkjang/invenqor/server/internal/config"
	"github.com/hkjang/invenqor/server/internal/httpapi"
	"github.com/hkjang/invenqor/server/internal/ingest"
	"github.com/hkjang/invenqor/server/internal/spool"
	"github.com/hkjang/invenqor/server/internal/storage"
	"github.com/hkjang/invenqor/server/internal/updates"
	"github.com/hkjang/invenqor/server/internal/version"
)

func main() {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{
		Level: slog.LevelInfo,
	}))
	if err := run(logger); err != nil {
		logger.Error("server_stopped", "error", err)
		os.Exit(1)
	}
}

func run(logger *slog.Logger) error {
	processConfig, err := config.Load()
	if err != nil {
		return err
	}
	bootstrapStore, err := bootstrap.OpenWithKey(
		processConfig.StateDir,
		processConfig.MasterKeyPath,
	)
	if err != nil {
		return err
	}
	bootstrapValues, err := bootstrapStore.Load()
	if err != nil {
		return err
	}
	if processConfig.PostgresDSN == "" {
		processConfig.PostgresDSN = bootstrapValues.PostgresDSN
	}
	if bootstrapValues.SQLitePath != "" &&
		processConfig.SQLitePath == processConfig.StateDir+"/invenqor.db" {
		processConfig.SQLitePath = bootstrapValues.SQLitePath
	}
	if !bootstrapStore.Exists() {
		if err := bootstrapStore.Save(bootstrap.Values{
			PostgresDSN: processConfig.PostgresDSN,
			SQLitePath:  processConfig.SQLitePath,
		}); err != nil {
			return err
		}
	}

	rootContext, stop := signal.NotifyContext(
		context.Background(),
		os.Interrupt,
		syscall.SIGTERM,
	)
	defer stop()

	database, err := storage.Open(rootContext, storage.Options{
		PostgresDSN: processConfig.PostgresDSN,
		SQLitePath:  processConfig.SQLitePath,
		Schema:      processConfig.DatabaseSchema,
		Timeout:     processConfig.DatabaseTimeout,
	})
	if err != nil {
		return err
	}
	defer database.Close()

	totpService := auth.NewTOTPService(database.DB(), bootstrapStore)
	authOptions := auth.DefaultServiceOptions()
	authOptions.TOTP = totpService
	authService, err := auth.NewService(database.DB(), authOptions)
	if err != nil {
		return err
	}
	bootstrapManager := auth.NewBootstrapManager(database.DB(), processConfig.StateDir)
	oidcService := auth.NewOIDCService(database.DB(), bootstrapStore, authService)
	agentService := agents.NewService(database.DB())
	ingestService := ingest.NewService(database.DB())
	eventSpool, err := spool.Open(processConfig.StateDir)
	if err != nil {
		return err
	}
	updateStore, err := updates.Open(processConfig.UpdateDir)
	if err != nil {
		return err
	}
	apiKeyService := apikeys.NewService(database.DB())
	bootstrapStatus, err := bootstrapManager.Ensure(rootContext)
	if err != nil {
		return err
	}
	if bootstrapStatus.Required {
		logger.Warn(
			"initial_setup_required",
			"bootstrap_token_file", bootstrapStatus.TokenFile,
		)
	}

	api := httpapi.New(httpapi.Options{
		Database:         database,
		AuthService:      authService,
		OIDCService:      oidcService,
		TOTPService:      totpService,
		BootstrapManager: bootstrapManager,
		AgentService:     agentService,
		IngestService:    ingestService,
		Spool:            eventSpool,
		BootstrapStore:   bootstrapStore,
		UpdateStore:      updateStore,
		APIKeyService:    apiKeyService,
		Logger:           logger,
	})
	go api.RunSpoolReplay(rootContext, 5*time.Second)
	httpServer := &http.Server{
		Addr:              processConfig.ListenAddress,
		Handler:           api.Handler(),
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      30 * time.Second,
		IdleTimeout:       2 * time.Minute,
	}
	serverErrors := make(chan error, 1)
	go func() {
		logger.Info(
			"server_started",
			"version", version.Version,
			"listen_address", processConfig.ListenAddress,
			"database_mode", database.Mode(),
		)
		if failure := database.PostgresFailure(); failure != nil {
			logger.Warn(
				"postgres_startup_failed",
				"code", failure.Code,
				"summary", failure.Summary,
				"host", failure.Host,
			)
		}
		serverErrors <- httpServer.ListenAndServe()
	}()

	select {
	case <-rootContext.Done():
	case err := <-serverErrors:
		if !errors.Is(err, http.ErrServerClosed) {
			return err
		}
	}
	shutdownContext, cancel := context.WithTimeout(
		context.Background(),
		processConfig.ShutdownTimeout,
	)
	defer cancel()
	if err := httpServer.Shutdown(shutdownContext); err != nil {
		return err
	}
	logger.Info("server_shutdown_complete")
	return nil
}
