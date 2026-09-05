package storage

import (
	"go/ast"
	"go/parser"
	"go/token"
	"io/fs"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"testing"
)

// A loop over *sql.Rows stops for two reasons it cannot tell apart: the result
// was read to the end, or the iteration failed part way through - the
// connection dropped, the statement was cancelled, a value would not decode.
// Next() returns false for both and rows.Err() is the only thing that
// separates them, so a loop without it reports whatever rows happened to
// arrive as the whole answer. Here that is an asset query returning three
// hosts out of three thousand, with HTTP 200 and nothing in the response
// saying it is a fragment.
//
// Some call sites already checked it and some did not, which is the kind of
// difference review does not catch by reading one handler at a time, so this
// reads every one of them.
func TestEveryRowIterationChecksTheIterationError(t *testing.T) {
	// The convention is one function wide: a loop over rows.Next() must be
	// followed, somewhere in the function that opened them, by rows.Err() on
	// the same variable. A function iterating two cursors of the same name is
	// counted as satisfied by a single check; naming them apart, as the two
	// call sites that iterate a second cursor already do, keeps that exact.
	set := token.NewFileSet()
	loops := 0
	var offenders []string

	root := filepath.Join("..", "..")
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() || !strings.HasSuffix(path, ".go") ||
			strings.HasSuffix(path, "_test.go") {
			return nil
		}
		parsed, parseErr := parser.ParseFile(set, path, nil, 0)
		if parseErr != nil {
			return parseErr
		}
		for _, declaration := range parsed.Decls {
			function, isFunction := declaration.(*ast.FuncDecl)
			if !isFunction || function.Body == nil {
				continue
			}
			checked := receiversOfCall(function.Body, "Err")
			for cursor := range receiversOfCall(function.Body, "Next") {
				loops++
				if checked[cursor] {
					continue
				}
				position := set.Position(function.Pos())
				offenders = append(offenders, strings.Join([]string{
					filepath.ToSlash(path), ":", strconv.Itoa(position.Line), " ",
					function.Name.Name, "() iterates ", cursor,
					" without checking ", cursor, ".Err()",
				}, ""))
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}

	if loops == 0 {
		t.Fatal("no row iterations were found, so this test proves nothing")
	}
	if len(offenders) > 0 {
		sort.Strings(offenders)
		t.Errorf(
			"these row iterations report a failed iteration as a complete "+
				"result:\n  %s\n\nCheck rows.Err() after the loop and fail the "+
				"request rather than answering with the rows that arrived.",
			strings.Join(offenders, "\n  "),
		)
	}
}

// receiversOfCall reports the variables a no-argument method of the given name
// is called on anywhere inside the body, including in closures.
func receiversOfCall(body *ast.BlockStmt, method string) map[string]bool {
	found := map[string]bool{}
	ast.Inspect(body, func(node ast.Node) bool {
		call, isCall := node.(*ast.CallExpr)
		if !isCall || len(call.Args) != 0 {
			return true
		}
		selector, isSelector := call.Fun.(*ast.SelectorExpr)
		if !isSelector || selector.Sel.Name != method {
			return true
		}
		if receiver, isIdent := selector.X.(*ast.Ident); isIdent {
			found[receiver.Name] = true
		}
		return true
	})
	return found
}
