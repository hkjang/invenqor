package config

import (
	"go/ast"
	"go/parser"
	"go/token"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"testing"
)

var environmentName = regexp.MustCompile(`INVENQOR_[A-Z0-9_]+`)

// Guidance shown to an operator must name a variable that exists.
//
// The KEYCLOAK_SECRET_UNREADABLE diagnostic told operators to "set
// INVENQOR_MASTER_KEY to the same value on every instance". There is no such
// variable - the real one is INVENQOR_MASTER_KEY_FILE and it takes a path.
// Someone following that instruction sees no change and has no reason to doubt
// it, which is worse than being told nothing.
//
// The rule that catches it: a name written inside a sentence must also appear
// somewhere on its own, as the literal passed to whatever reads it. A name that
// only ever appears inside prose is a name nothing reads.
func TestEnvironmentVariablesNamedInGuidanceAreRead(t *testing.T) {
	root := repositoryRoot(t)
	declared := map[string]bool{}
	mentions := map[string][]string{}

	forEachStringLiteral(t, root, func(file string, line int, value string) {
		names := environmentName.FindAllString(value, -1)
		if len(names) == 0 {
			return
		}
		// The literal is exactly one variable name: this is a read site.
		if len(names) == 1 && strings.TrimSpace(value) == names[0] {
			declared[names[0]] = true
			return
		}
		for _, name := range names {
			mentions[name] = append(
				mentions[name],
				filepath.Base(file)+":"+strconv.Itoa(line),
			)
		}
	})

	if len(declared) == 0 {
		t.Fatal("no environment variables were found, so this test proves nothing")
	}
	for name, where := range mentions {
		if !declared[name] {
			t.Errorf(
				"%s is named in guidance at %s but nothing reads it;\n"+
					"an operator following that instruction would see no effect",
				name, strings.Join(where, ", "),
			)
		}
	}
}

func repositoryRoot(t *testing.T) string {
	t.Helper()
	// This package sits at server/internal/config.
	root, err := filepath.Abs(filepath.Join("..", ".."))
	if err != nil {
		t.Fatal(err)
	}
	return root
}

func forEachStringLiteral(
	t *testing.T,
	root string,
	visit func(file string, line int, value string),
) {
	t.Helper()
	files, err := filepath.Glob(filepath.Join(root, "internal", "*", "*.go"))
	if err != nil {
		t.Fatal(err)
	}
	more, err := filepath.Glob(filepath.Join(root, "cmd", "*", "*.go"))
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range append(files, more...) {
		if strings.HasSuffix(path, "guidance_test.go") {
			continue
		}
		set := token.NewFileSet()
		parsed, err := parser.ParseFile(set, path, nil, 0)
		if err != nil {
			t.Fatalf("parse %s: %v", path, err)
		}
		ast.Inspect(parsed, func(node ast.Node) bool {
			literal, ok := node.(*ast.BasicLit)
			if !ok || literal.Kind != token.STRING {
				return true
			}
			value, err := strconv.Unquote(literal.Value)
			if err != nil {
				return true
			}
			visit(path, set.Position(literal.Pos()).Line, value)
			return true
		})
	}
}
