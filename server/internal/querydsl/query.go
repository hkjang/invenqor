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
		field := strings.ToLower(match[1])
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
			if strings.HasPrefix(clause.Value, "now - ") {
				durationText := strings.Trim(
					strings.TrimPrefix(clause.Value, "now - "), `"`,
				)
				duration, err := time.ParseDuration(durationText)
				if err != nil {
					return "", nil, errors.New("relative time duration is invalid")
				}
				value = time.Now().UTC().Add(-duration)
			}
		}
		args = append(args, value)
		conditions = append(
			conditions,
			fmt.Sprintf("%s %s $%d", column, clause.Operator, len(args)),
		)
	}
	return strings.Join(conditions, " AND "), args, nil
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
	{"first_seen_at", "time", "최초 확인 시각", `first_seen_at >= "now - 168h"`},
	{"last_seen_at", "time", "최근 확인 시각", `last_seen_at < "now - 24h"`},
	{
		"attributes.*", "path",
		"수집 속성 경로. 예: attributes.os_name",
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

func allowedField(field string) bool {
	for _, candidate := range fields {
		if candidate.Name == field {
			return true
		}
	}
	return strings.HasPrefix(field, "attributes.") &&
		safePath.MatchString(strings.TrimPrefix(field, "attributes."))
}

func columnFor(field string, postgres bool) (string, error) {
	if !allowedField(field) {
		return "", fmt.Errorf("field %q is not allowed", field)
	}
	if !strings.HasPrefix(field, "attributes.") {
		return field, nil
	}
	path := strings.Split(strings.TrimPrefix(field, "attributes."), ".")
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
