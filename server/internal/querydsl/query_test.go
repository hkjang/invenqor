package querydsl

import (
	"strings"
	"testing"
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
