package querydsl

import (
	"errors"
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"time"
)

var (
	clausePattern = regexp.MustCompile(
		`(?i)^\s*([a-z_][a-z0-9_.]*)\s*(=|!=|<=|>=|<|>)\s*(?:"([^"]*)"|'([^']*)'|([a-z0-9_.:@/+ -]+))\s*$`,
	)
	safePath = regexp.MustCompile(`^[a-zA-Z_][a-zA-Z0-9_.]*$`)
)

const (
	maxClauses = 20
	maxLength  = 4096
)

type Clause struct {
	Field    string `json:"field"`
	Operator string `json:"operator"`
	Value    string `json:"value"`
}

type Query struct {
	Clauses []Clause `json:"clauses"`
}

func Parse(input string) (Query, error) {
	input = strings.TrimSpace(input)
	if input == "" {
		return Query{}, errors.New("query is empty")
	}
	if len(input) > maxLength {
		return Query{}, fmt.Errorf("query exceeds %d characters", maxLength)
	}
	parts, err := splitAND(input)
	if err != nil {
		return Query{}, err
	}
	if len(parts) > maxClauses {
		return Query{}, fmt.Errorf("query has more than %d clauses", maxClauses)
	}
	result := Query{Clauses: make([]Clause, 0, len(parts))}
	for _, part := range parts {
		match := clausePattern.FindStringSubmatch(part)
		if match == nil {
			return Query{}, fmt.Errorf("invalid clause %q", strings.TrimSpace(part))
		}
		field := normalizeField(match[1])
		if !allowedField(field) {
			return Query{}, fmt.Errorf("field %q is not allowed", field)
		}
		value := match[3]
		if value == "" {
			value = match[4]
		}
		if value == "" {
			value = strings.TrimSpace(match[5])
		}
		result.Clauses = append(result.Clauses, Clause{
			Field: field, Operator: match[2], Value: value,
		})
	}
	return result, nil
}

func (q Query) SQL(postgres bool) (string, []any, error) {
	if len(q.Clauses) == 0 {
		return "", nil, errors.New("query has no clauses")
	}
	conditions := []string{"deleted_at IS NULL"}
	args := make([]any, 0, len(q.Clauses))
	for _, clause := range q.Clauses {
		column, err := columnFor(clause.Field, postgres)
		if err != nil {
			return "", nil, err
		}
		value := any(clause.Value)
		if clause.Field == "confidence" {
			number, err := strconv.ParseFloat(clause.Value, 64)
			if err != nil {
				return "", nil, errors.New("confidence must be numeric")
			}
			value = number
		}
		if clause.Field == "last_seen_at" || clause.Field == "first_seen_at" {
			moment, err := parseTime(clause.Value)
			if err != nil {
				return "", nil, err
			}
			value = moment
		}
		args = append(args, value)
		conditions = append(
			conditions,
			fmt.Sprintf("%s %s $%d", column, clause.Operator, len(args)),
		)
	}
	return strings.Join(conditions, " AND "), args, nil
}

// timeLayouts are the absolute forms a time clause may name, most specific
// first. A form without a zone is read as UTC, which is the zone every stored
// timestamp is written in.
var timeLayouts = []string{
	time.RFC3339Nano,
	"2006-01-02 15:04:05.999999999Z07:00",
	"2006-01-02 15:04:05.999999999 -0700 MST",
	"2006-01-02T15:04:05.999999999",
	"2006-01-02 15:04:05.999999999",
	"2006-01-02",
}

// parseTime resolves a time clause's value to an instant. Only the relative
// "now - 24h" form was resolved before and anything else was handed to the
// database as text, which was wrong in both storage modes. PostgreSQL compares
// TIMESTAMPTZ, so a value it could not read as a timestamp failed the statement
// and the operator got HTTP 500 with no hint of the typo. The SQLite fallback
// stores the column as text written from a Go time, so "2026-01-01T00:00:00Z"
// was compared byte by byte against "2026-01-01 00:00:00 +0000 UTC" - no error,
// just the wrong rows.
func parseTime(value string) (time.Time, error) {
	text := strings.TrimSpace(value)
	if strings.EqualFold(text, "now") {
		return time.Now().UTC(), nil
	}
	if rest, found := cutRelativeNow(text); found {
		duration, err := time.ParseDuration(strings.TrimSpace(rest))
		if err != nil {
			return time.Time{}, errors.New("relative time duration is invalid")
		}
		return time.Now().UTC().Add(-duration), nil
	}
	for _, layout := range timeLayouts {
		if moment, err := time.Parse(layout, text); err == nil {
			return moment.UTC(), nil
		}
	}
	return time.Time{}, fmt.Errorf(
		"time value %q is neither %s nor a timestamp such as "+
			`"2026-01-31T09:00:00Z"`,
		text, `"now - 24h"`,
	)
}

func cutRelativeNow(text string) (string, bool) {
	if len(text) < 4 || !strings.EqualFold(text[:3], "now") {
		return "", false
	}
	rest := strings.TrimSpace(text[3:])
	if !strings.HasPrefix(rest, "-") {
		return "", false
	}
	return strings.TrimPrefix(rest, "-"), true
}

// Field describes one queryable field for the console's reference panel. The
// grammar was only discoverable by trial and error: an operator had to guess a
// field name, read "field ... is not allowed", and guess again.
type Field struct {
	Name        string `json:"name"`
	Kind        string `json:"kind"`
	Description string `json:"description"`
	Example     string `json:"example"`
}

// fields is the single source for both the parser's allowlist and the published
// reference, so the two cannot disagree.
var fields = []Field{
	{"name", "text", "자산 이름", `name = "web-01"`},
	{"asset_key", "text", "수집 원천이 부여한 고유 키", `asset_key = "host:web-01"`},
	{"id", "text", "자산 UUID", `id = "0d0f…"`},
	{"type", "text", "자산 유형", `type = "host"`},
	{"status", "text", "수명주기 상태", `status = "active"`},
	{"environment", "text", "분류가 판정한 운영 환경", `environment = "production"`},
	{"criticality", "text", "분류가 판정한 중요도", `criticality = "critical"`},
	{"owner_department", "text", "담당 부서", `owner_department = "플랫폼"`},
	{"location", "text", "위치", `location = "IDC-1"`},
	{"source", "text", "수집 원천", `source = "agent"`},
	{"confidence", "number", "분류 확신도 0~1", "confidence >= 0.8"},
	{
		"first_seen_at", "time",
		`최초 확인 시각. "now - 168h" 또는 "2026-01-31T09:00:00Z"`,
		`first_seen_at >= "now - 168h"`,
	},
	{
		"last_seen_at", "time",
		`최근 확인 시각. "now - 24h" 또는 "2026-01-31T09:00:00Z"`,
		`last_seen_at < "now - 24h"`,
	},
	{
		"attributes.*", "path",
		"수집 속성 경로. 대소문자를 구분합니다. 예: attributes.os_name",
		`attributes.os_name = "Ubuntu"`,
	},
}

// Grammar is everything the console needs to explain the query language.
type Grammar struct {
	Fields      []Field  `json:"fields"`
	Operators   []string `json:"operators"`
	Combinator  string   `json:"combinator"`
	MaxClauses  int      `json:"max_clauses"`
	MaxLength   int      `json:"max_length"`
	RelativeNow string   `json:"relative_now"`
}

func Describe() Grammar {
	return Grammar{
		Fields:      fields,
		Operators:   []string{"=", "!=", "<", "<=", ">", ">="},
		Combinator:  "AND",
		MaxClauses:  maxClauses,
		MaxLength:   maxLength,
		RelativeNow: `"now - 24h"`,
	}
}

const attributePrefix = "attributes."

// normalizeField folds a field name to the spelling the grammar uses. A column
// is named case-insensitively, but everything after "attributes." is a key in
// the stored JSON document and JSON keys are case sensitive. Folding the whole
// field lowercased that key too, so "attributes.assetTag" asked the document
// for "assettag", a key nobody wrote, and the query answered with no rows and
// no error - and /query/validate had already called the expression valid.
func normalizeField(field string) string {
	if len(field) > len(attributePrefix) &&
		strings.EqualFold(field[:len(attributePrefix)], attributePrefix) {
		return attributePrefix + field[len(attributePrefix):]
	}
	return strings.ToLower(field)
}

func allowedField(field string) bool {
	for _, candidate := range fields {
		if candidate.Name == field {
			return true
		}
	}
	return strings.HasPrefix(field, attributePrefix) &&
		safePath.MatchString(strings.TrimPrefix(field, attributePrefix))
}

func columnFor(field string, postgres bool) (string, error) {
	if !allowedField(field) {
		return "", fmt.Errorf("field %q is not allowed", field)
	}
	if !strings.HasPrefix(field, attributePrefix) {
		return field, nil
	}
	path := strings.Split(strings.TrimPrefix(field, attributePrefix), ".")
	if postgres {
		return "attributes_json #>> '{" + strings.Join(path, ",") + "}'", nil
	}
	return "json_extract(attributes_json, '$." + strings.Join(path, ".") + "')", nil
}

func splitAND(input string) ([]string, error) {
	parts := make([]string, 0)
	start := 0
	var quote rune
	runes := []rune(input)
	for index := 0; index < len(runes); index++ {
		character := runes[index]
		if quote != 0 {
			if character == quote {
				quote = 0
			}
			continue
		}
		if character == '"' || character == '\'' {
			quote = character
			continue
		}
		if index+3 <= len(runes) &&
			strings.EqualFold(string(runes[index:index+3]), "AND") &&
			(index == 0 || isSpace(runes[index-1])) &&
			(index+3 == len(runes) || isSpace(runes[index+3])) {
			parts = append(parts, string(runes[start:index]))
			start = index + 3
			index += 2
		}
	}
	if quote != 0 {
		return nil, errors.New("query contains an unterminated string")
	}
	parts = append(parts, string(runes[start:]))
	return parts, nil
}

func isSpace(character rune) bool {
	return character == ' ' || character == '\t' ||
		character == '\r' || character == '\n'
}
