package main_test

import (
	"fmt"
	"go/ast"
	"go/importer"
	"go/parser"
	"go/token"
	"go/types"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

// Check the real typed syntax tree, so renaming enums or adding a defensive
// default cannot silently disable exhaustiveness. Adding a variant requires
// every switch on its type to handle it explicitly.
func checkExhaustiveness(fset *token.FileSet, files []*ast.File) error {
	info := &types.Info{Types: map[ast.Expr]types.TypeAndValue{}}
	pkg, err := (&types.Config{Importer: importer.Default()}).Check("prototype", fset, files, info)
	if err != nil {
		return err
	}
	enums := map[types.Type]map[string]string{}
	for _, name := range pkg.Scope().Names() {
		c, ok := pkg.Scope().Lookup(name).(*types.Const)
		if !ok {
			continue
		}
		named, ok := c.Type().(*types.Named)
		if !ok || named.Obj().Pkg() != pkg {
			continue
		}
		if enums[c.Type()] == nil {
			enums[c.Type()] = map[string]string{}
		}
		enums[c.Type()][c.Val().ExactString()] = c.Name()
	}
	var missing []string
	uses := map[types.Type]int{}
	for _, file := range files {
		ast.Inspect(file, func(node ast.Node) bool {
			sw, ok := node.(*ast.SwitchStmt)
			if !ok || sw.Tag == nil {
				return true
			}
			typ := info.Types[sw.Tag].Type
			variants := enums[typ]
			if len(variants) == 0 {
				return true
			}
			uses[typ]++
			covered := map[string]bool{}
			for _, item := range sw.Body.List {
				for _, expr := range item.(*ast.CaseClause).List {
					if value := info.Types[expr].Value; value != nil {
						covered[value.ExactString()] = true
					}
				}
			}
			for value, name := range variants {
				if !covered[value] {
					missing = append(missing, fmt.Sprintf("%s: missing %s", fset.Position(sw.Pos()), name))
				}
			}
			return true
		})
	}
	for typ := range enums {
		if uses[typ] == 0 {
			missing = append(missing, "enum has no exhaustive switch: "+typ.String())
		}
	}
	sort.Strings(missing)
	if len(missing) > 0 {
		return fmt.Errorf("%s", strings.Join(missing, "\n"))
	}
	return nil
}

func TestStateExhaustiveness(t *testing.T) {
	fset := token.NewFileSet()
	paths, err := filepath.Glob("*.go")
	if err != nil {
		t.Fatal(err)
	}
	var files []*ast.File
	for _, path := range paths {
		if strings.HasSuffix(path, "_test.go") {
			continue
		}
		file, err := parser.ParseFile(fset, path, nil, 0)
		if err != nil {
			t.Fatal(err)
		}
		files = append(files, file)
	}
	if err := checkExhaustiveness(fset, files); err != nil {
		t.Fatal(err)
	}
}

func TestExhaustivenessRejectsUnhandledVariant(t *testing.T) {
	// These fixtures are the checker seam: both omitted existing cases and a
	// newly added state must fail, including switches with a catch-all default.
	for _, source := range []string{
		`package fixture; type state uint8; const (green state = iota; red); func f(s state) { switch s { case green:; default: } }`,
		`package fixture; type state uint8; const (green state = iota; red; added); func f(s state) { switch s { case green, red:; default: } }`,
	} {
		fset := token.NewFileSet()
		file, err := parser.ParseFile(fset, "fixture.go", source, 0)
		if err != nil {
			t.Fatal(err)
		}
		if err := checkExhaustiveness(fset, []*ast.File{file}); err == nil || !strings.Contains(err.Error(), "missing") {
			t.Fatalf("unhandled variant accepted: %v", err)
		}
	}
	// Also ensure a complete switch is accepted by the same checker.
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, "complete.go", `package fixture; type state uint8; const (green state = iota; red); func f(s state) { switch s { case green, red: } }`, 0)
	if err != nil {
		t.Fatal(err)
	}
	if err := checkExhaustiveness(fset, []*ast.File{file}); err != nil {
		t.Fatal(err)
	}
}
