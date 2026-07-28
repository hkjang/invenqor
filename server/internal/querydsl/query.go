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
	if len(input) > 4096 {
		return Query{}, errors.New("query exceeds 4096 characters")
	}
	parts, err := splitAND(input)
	if err != nil {
		return Query{}, err
	}
	if len(parts) > 20 {
		return Query{}, errors.New("query has more than 20 clauses")
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

func allowedField(field string) bool {
	switch field {
	case "id", "asset_key", "name", "type", "status", "criticality",
		"environment", "owner_department", "location", "source",
		"confidence", "first_seen_at", "last_seen_at":
		return true
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
