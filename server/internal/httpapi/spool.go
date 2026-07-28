package httpapi

import (
	"context"
	"time"

	"github.com/hkjang/invenqor/server/internal/ingest"
	"github.com/hkjang/invenqor/server/internal/storage"
)

// RunSpoolReplay continually checks the existing PostgreSQL connection. It
// never opens SQLite or changes database handles during a runtime outage.
func (s *Server) RunSpoolReplay(ctx context.Context, interval time.Duration) {
	if s.spool == nil {
		return
	}
	if interval <= 0 {
		interval = 5 * time.Second
	}
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		if s.database.Mode() != storage.ModePostgresDegraded {
			if count, err := s.ReplaySpool(ctx); err != nil {
				s.logger.Warn("event_spool_replay_failed", "error", err)
			} else if count > 0 {
				s.logger.Info("event_spool_replayed", "events", count)
			}
		} else {
			pingContext, cancel := context.WithTimeout(ctx, 2*time.Second)
			err := s.database.Ping(pingContext)
			cancel()
			if err == nil {
				s.logger.Info("postgres_connection_recovered")
			}
		}
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
}

func (s *Server) ReplaySpool(ctx context.Context) (int, error) {
	if s.spool == nil {
		return 0, nil
	}
	paths, err := s.spool.Pending()
	if err != nil {
		return 0, err
	}
	replayed := 0
	for _, path := range paths {
		raw, err := s.spool.Read(path)
		if err != nil {
			return replayed, err
		}
		envelope, err := ingest.DecodeEnvelope(raw)
		if err != nil {
			s.logger.Error(
				"invalid_spooled_event",
				"path", path,
				"error", err,
			)
			continue
		}
		agent, err := s.agentService.GetByExternalID(ctx, envelope.AgentID)
		if err != nil {
			return replayed, err
		}
		if _, err := s.ingestService.Process(
			ctx, agent, envelope, raw, "",
		); err != nil {
			return replayed, err
		}
		if err := s.spool.Acknowledge(path); err != nil {
			return replayed, err
		}
		replayed++
	}
	return replayed, nil
}
