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

// An attribute path names a key in a JSON document, and JSON keys are case
// sensitive. The parser folded the whole field to lower case, so a query for an
// attribute written with a capital - every attribute on an asset created
// through the API or imported from another system - asked for a key nobody
// wrote and came back empty with no error.
func TestAttributePathKeepsItsCase(t *testing.T) {
	for _, testCase := range []struct {
		input    string
		postgres string
		sqlite   string
	}{
		{`attributes.assetTag = "AT-1"`,
			"attributes_json #>> '{assetTag}'",
			"json_extract(attributes_json, '$.assetTag')"},
		{`ATTRIBUTES.OS.Family = "rhel"`,
			"attributes_json #>> '{OS,Family}'",
			"json_extract(attributes_json, '$.OS.Family')"},
	} {
		query, err := Parse(testCase.input)
		if err != nil {
			t.Fatalf("Parse(%q) error = %v", testCase.input, err)
		}
		where, _, err := query.SQL(true)
		if err != nil {
			t.Fatalf("SQL(%q) error = %v", testCase.input, err)
		}
		if !strings.Contains(where, testCase.postgres) {
			t.Fatalf("%q compiled to %s, want %s",
				testCase.input, where, testCase.postgres)
		}
		where, _, err = query.SQL(false)
		if err != nil {
			t.Fatalf("SQL(%q) error = %v", testCase.input, err)
		}
		if !strings.Contains(where, testCase.sqlite) {
			t.Fatalf("%q compiled to %s, want %s",
				testCase.input, where, testCase.sqlite)
		}
	}
}

// Column names stay case-insensitive: only the JSON key after "attributes."
// carries meaning in its case.
func TestColumnFieldsStayCaseInsensitive(t *testing.T) {
	query, err := Parse(`Type = "host" AND ENVIRONMENT = "production"`)
	if err != nil {
		t.Fatal(err)
	}
	if query.Clauses[0].Field != "type" ||
		query.Clauses[1].Field != "environment" {
		t.Fatalf("clauses = %#v", query.Clauses)
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

// An attribute is extracted from the document as text, so an ordering
// comparison compared digit strings: "16000000000" sorts before "2000000000"
// because '1' < '2', and a host with 16 GB fell outside "at least 2 GB".
func TestOrderingComparesAStoredNumberAsANumber(t *testing.T) {
	query, err := Parse(`attributes.memory_bytes >= 2000000000`)
	if err != nil {
		t.Fatal(err)
	}
	for _, testCase := range []struct {
		postgres bool
		want     string
	}{
		{true, "(CASE WHEN jsonb_typeof(attributes_json #> '{memory_bytes}') " +
			"= 'number' THEN (attributes_json #>> '{memory_bytes}')" +
			"::double precision >= $1 ELSE attributes_json #>> " +
			"'{memory_bytes}' >= $2 END)"},
		{false, "(CASE WHEN json_type(attributes_json, '$.memory_bytes') " +
			"IN ('integer','real') THEN json_extract(attributes_json, " +
			"'$.memory_bytes') >= $1 ELSE json_extract(attributes_json, " +
			"'$.memory_bytes') >= $2 END)"},
	} {
		where, args, err := query.SQL(testCase.postgres)
		if err != nil {
			t.Fatalf("SQL(%v) error = %v", testCase.postgres, err)
		}
		if !strings.Contains(where, testCase.want) {
			t.Fatalf("SQL(%v) = %s, want %s",
				testCase.postgres, where, testCase.want)
		}
		if len(args) != 2 || args[0] != 2000000000.0 ||
			args[1] != "2000000000" {
			t.Fatalf("args = %#v", args)
		}
	}
}

// Only an ordering comparison reads the value as a number. Equality is already
// right for text, and reading "1.10" as a number there would stop an attribute
// holding a version from matching itself.
func TestEqualityAndTextValuesStayTextComparisons(t *testing.T) {
	for _, input := range []string{
		`attributes.cpu_count = 4`,
		`attributes.cpu_count != 4`,
		`attributes.os_version > "20.04.1"`,
	} {
		query, err := Parse(input)
		if err != nil {
			t.Fatalf("Parse(%q) error = %v", input, err)
		}
		where, args, err := query.SQL(true)
		if err != nil {
			t.Fatalf("SQL(%q) error = %v", input, err)
		}
		if strings.Contains(where, "CASE") || len(args) != 1 {
			t.Fatalf("%q compiled to %s with args %#v", input, where, args)
		}
	}
}
