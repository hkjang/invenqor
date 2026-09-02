package querydsl

import (
	"strings"
	"testing"
	"time"
)

func TestParseAndCompileUsesBoundParameters(t *testing.T) {
	query, err := Parse(
		`type = "host" AND environment = "production" AND attributes.os.family = "rhel"`,
	)
	if err != nil {
		t.Fatal(err)
	}
	where, args, err := query.SQL(true)
	if err != nil {
		t.Fatal(err)
	}
	if len(args) != 3 || args[0] != "host" || args[2] != "rhel" {
		t.Fatalf("args = %#v", args)
	}
	if strings.Contains(where, "production") || strings.Contains(where, "rhel") {
		t.Fatalf("values leaked into SQL: %s", where)
	}
	if !strings.Contains(where, "attributes_json #>> '{os,family}'") {
		t.Fatalf("PostgreSQL JSON path missing: %s", where)
	}
}

func TestRejectsSQLAndUnknownFields(t *testing.T) {
	for _, input := range []string{
		`type = "host"; DROP TABLE assets`,
		`password_hash = "x"`,
		`type = "host" OR 1 = 1`,
		`attributes.bad-path = "x"`,
	} {
		if _, err := Parse(input); err == nil {
			t.Fatalf("Parse(%q) accepted unsafe input", input)
		}
	}
}

// A time clause has to reach the database as an instant. Text reached
// PostgreSQL as text and failed the whole statement, and reached the SQLite
// fallback as a byte-by-byte comparison against a differently formatted column.
func TestTimeClausesCompileToInstants(t *testing.T) {
	for _, testCase := range []struct {
		input string
		want  time.Time
	}{
		{`last_seen_at < "2026-01-31T09:00:00Z"`,
			time.Date(2026, 1, 31, 9, 0, 0, 0, time.UTC)},
		{`first_seen_at >= "2026-01-31 09:00:00"`,
			time.Date(2026, 1, 31, 9, 0, 0, 0, time.UTC)},
		{`first_seen_at >= "2026-01-31"`,
			time.Date(2026, 1, 31, 0, 0, 0, 0, time.UTC)},
	} {
		query, err := Parse(testCase.input)
		if err != nil {
			t.Fatalf("Parse(%q) error = %v", testCase.input, err)
		}
		_, args, err := query.SQL(true)
		if err != nil {
			t.Fatalf("SQL(%q) error = %v", testCase.input, err)
		}
		moment, ok := args[0].(time.Time)
		if !ok {
			t.Fatalf("%q compiled to %#v, want a time", testCase.input, args[0])
		}
		if !moment.Equal(testCase.want) {
			t.Fatalf("%q compiled to %s, want %s",
				testCase.input, moment, testCase.want)
		}
	}
}

func TestRelativeTimeStaysSupported(t *testing.T) {
	query, err := Parse(`last_seen_at < "now - 24h"`)
	if err != nil {
		t.Fatal(err)
	}
	_, args, err := query.SQL(true)
	if err != nil {
		t.Fatal(err)
	}
	moment, ok := args[0].(time.Time)
	if !ok {
		t.Fatalf("args[0] = %#v, want a time", args[0])
	}
	if elapsed := time.Since(moment); elapsed < 23*time.Hour ||
		elapsed > 25*time.Hour {
		t.Fatalf("now - 24h resolved to %s, %s ago", moment, elapsed)
	}
}

// An unreadable time is the operator's typo, not a server fault, so it has to
// be refused where the caller still gets to see the value that was rejected.
func TestUnreadableTimeIsRefusedBeforeItReachesTheDatabase(t *testing.T) {
	for _, input := range []string{
		`last_seen_at < "yesterday"`,
		`last_seen_at < "now - two days"`,
		`first_seen_at >= "2026-13-45"`,
		`first_seen_at >= "2026-01-31T25:00:00Z"`,
		`last_seen_at < "now -"`,
	} {
		query, err := Parse(input)
		if err != nil {
			continue
		}
		if _, _, err := query.SQL(true); err == nil {
			t.Fatalf("%q compiled instead of naming the bad time", input)
		}
	}
}
